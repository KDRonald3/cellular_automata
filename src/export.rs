use std::fs::{self, File, OpenOptions};
use std::io::{self, BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use base64::engine::general_purpose::STANDARD;
use base64::Engine as _;

use crate::sim::BoundaryMode;

// To keep exported HTML viewable for very large runs, the embedded preview is
// split into stacked canvases so each stays within safe dimensions. These caps
// bound the per-canvas size; stride stays at 1 (full fidelity).
const MAX_EXPORT_CELLS: usize = 200_000_000; // per-canvas cap, ~25 MB packed (~34 MB base64)
const MAX_EXPORT_HEIGHT: usize = 32_000; // per-canvas height cap

const MANIFEST_FILE: &str = "manifest.tsv";
const INDEX_FILE: &str = "index.html";
const DATA_DIR: &str = r"C:\Users\Ronald Kouatchoua\code\Research\Cellular Automata\Data";

pub struct ExportInput<'a> {
    pub job_id: u64,
    pub rule: u8,
    pub width: usize,
    pub generations: usize,
    pub progress: usize,
    pub boundary: BoundaryMode,
    pub status: &'a str,
    /// Flat row-major grid: cell (y, x) lives at `rows[y * width + x]`.
    pub rows: &'a [u8],
}

pub fn export_job(input: &ExportInput) -> io::Result<PathBuf> {
    let dir = runs_dir()?;
    fs::create_dir_all(&dir)?;

    let now = chrono::Local::now();
    let ts_file = now.format("%Y%m%d_%H%M%S").to_string();
    let ts_human = now.format("%Y-%m-%d %H:%M:%S").to_string();

    let boundary_short = match input.boundary {
        BoundaryMode::ZeroPadded => "zp",
        BoundaryMode::Wrap => "wr",
    };

    let filename = format!(
        "rule{:03}_w{}_g{}_{}_{}_job{}.html",
        input.rule, input.width, input.generations, boundary_short, ts_file, input.job_id
    );
    let path = dir.join(&filename);

    let html = render_run_html(input, &ts_human);
    fs::write(&path, html)?;

    append_manifest(&dir, input, &ts_human, &filename)?;
    regenerate_index(&dir)?;

    Ok(path)
}

fn render_run_html(input: &ExportInput, exported_at: &str) -> String {
    let rows = input.rows;
    let width = input.width;
    let height = if width == 0 { 0 } else { rows.len() / width };
    // Full-fidelity HTML preview: slice into stacked canvases so each stays
    // within safe dimensions while retaining stride == 1.
    let tile_max_rows_by_cells = if width == 0 {
        1
    } else {
        (MAX_EXPORT_CELLS / width).max(1)
    };
    let tile_max_rows = tile_max_rows_by_cells.min(MAX_EXPORT_HEIGHT).max(1);
    let num_tiles = if height == 0 {
        0
    } else {
        (height + tile_max_rows - 1) / tile_max_rows
    };

    let mut tile_meta_entries: Vec<String> = Vec::with_capacity(num_tiles);
    let mut tile_scripts = String::new();
    for tile_idx in 0..num_tiles {
        let start = tile_idx * tile_max_rows;
        let end = (start + tile_max_rows).min(height);
        let packed = pack_bits_range(rows, width, start, end);
        let b64 = STANDARD.encode(&packed);
        let id = format!("bits{}", tile_idx);
        tile_meta_entries.push(format!(
            "{{\"id\":\"{}\",\"start\":{},\"end\":{}}}",
            id, start, end
        ));
        tile_scripts.push_str(&format!(
            "<script id=\"{}\" type=\"application/octet-stream\">{}</script>\n",
            id, b64
        ));
    }
    let tiles_meta_json = tile_meta_entries.join(",");

    let title = format!(
        "Rule {} - {}x{} - job {}",
        input.rule, width, height, input.job_id
    );

    let mut html = String::with_capacity(tile_scripts.len() + 4096);
    html.push_str(&format!(
        "<!doctype html><html lang=\"en\"><head>\n\
         <meta charset=\"utf-8\">\n\
         <title>{}</title>\n\
         <style>\n\
         body {{ font-family: system-ui, sans-serif; margin: 20px; background: #fafafa; color: #222; }}\n\
         h1 {{ font-size: 18px; margin: 4px 0 12px 0; }}\n\
         dl {{ display: grid; grid-template-columns: auto 1fr; gap: 2px 12px; font-size: 13px; max-width: 420px; margin: 0 0 12px 0; }}\n\
         dt {{ color: #666; }}\n\
         canvas {{ image-rendering: pixelated; image-rendering: crisp-edges; border: 1px solid #ccc; display: block; margin-top: 12px; background: white; width: 100%; max-width: 1600px; height: auto; }}\n\
         a.back {{ font-size: 13px; color: #06c; text-decoration: none; }}\n\
         a.back:hover {{ text-decoration: underline; }}\n\
         </style>\n\
         </head><body>\n\
         <p><a class=\"back\" href=\"index.html\">&lt; back to index</a></p>\n\
         <h1>{}</h1>\n\
         <dl>\n\
           <dt>job id</dt><dd>{}</dd>\n\
           <dt>rule</dt><dd>{}</dd>\n\
           <dt>width</dt><dd>{}</dd>\n\
           <dt>generations</dt><dd>{} / {}</dd>\n\
           <dt>boundary</dt><dd>{}</dd>\n\
           <dt>status</dt><dd>{}</dd>\n\
           <dt>exported</dt><dd>{}</dd>\n\
           <dt>preview</dt><dd>{}</dd>\n\
         </dl>\n\
         <div id=\"tiles\"></div>\n\
         <script id=\"meta\" type=\"application/json\">{{\"w\":{},\"h\":{},\"tiles\":[{}]}}</script>\n\
         {}",
        html_escape(&title),
        html_escape(&title),
        input.job_id,
        input.rule,
        input.width,
        input.progress,
        input.generations,
        input.boundary,
        input.status,
        exported_at,
        if num_tiles <= 1 {
            "full fidelity (single canvas)".to_string()
        } else {
            format!(
                "full fidelity: {} tiles ({} rows each, last shorter)",
                num_tiles, tile_max_rows
            )
        },
        width.max(1),
        height.max(1),
        tiles_meta_json,
        tile_scripts,
    ));
    html.push_str(
         "<script>\n\
         (function(){\n\
          const meta = JSON.parse(document.getElementById('meta').textContent);\n\
          if (!meta || meta.w === 0 || meta.h === 0) return;\n\
          const host = document.getElementById('tiles');\n\
          for (const tile of meta.tiles || []) {\n\
            const b64 = (document.getElementById(tile.id)?.textContent || '').trim();\n\
            if (!b64) continue;\n\
            const bin = atob(b64);\n\
            const tileH = Math.max(0, (tile.end || 0) - (tile.start || 0));\n\
            if (tileH === 0) continue;\n\
            const cv = document.createElement('canvas');\n\
            cv.width = meta.w;\n\
            cv.height = tileH;\n\
            cv.style.imageRendering = 'pixelated';\n\
            cv.style.imageRendering = 'crisp-edges';\n\
            cv.style.display = 'block';\n\
            cv.style.marginTop = '12px';\n\
            cv.style.border = '1px solid #ccc';\n\
            const ctx = cv.getContext('2d');\n\
            const img = ctx.createImageData(meta.w, tileH);\n\
            for (let y = 0; y < tileH; y++) {\n\
              for (let x = 0; x < meta.w; x++) {\n\
                const iPacked = y * meta.w + x;\n\
                const bit = (bin.charCodeAt(iPacked >> 3) >> (7 - (iPacked & 7))) & 1;\n\
                const p   = iPacked * 4;\n\
                const v   = bit ? 0 : 255;\n\
                img.data[p] = v; img.data[p+1] = v; img.data[p+2] = v; img.data[p+3] = 255;\n\
              }\n\
            }\n\
            ctx.putImageData(img, 0, 0);\n\
            host.appendChild(cv);\n\
          }\n\
          if ((meta.tiles || []).length > 1) {\n\
            const note = document.createElement('p');\n\
            note.style.fontSize = '12px';\n\
            note.style.color = '#555';\n\
            note.textContent = `Rendered ${meta.tiles.length} stacked canvases (full fidelity, stride 1).`;\n\
            host.insertAdjacentElement('beforebegin', note);\n\
          }\n\
         })();\n\
         </script>\n\
         </body></html>\n",
    );

    html
}

fn pack_bits_range(rows: &[u8], width: usize, start_row: usize, end_row: usize) -> Vec<u8> {
    if width == 0 {
        return Vec::new();
    }
    let total_rows = rows.len().checked_div(width).unwrap_or(0);
    if start_row >= end_row || start_row >= total_rows {
        return Vec::new();
    }
    let clamped_end = end_row.min(total_rows);
    let rendered_h = clamped_end.saturating_sub(start_row);
    let total_bits = width.saturating_mul(rendered_h);
    let mut out = vec![0u8; total_bits.div_ceil(8)];

    for rendered_y in 0..rendered_h {
        let orig_y = start_row + rendered_y;
        let row = &rows[orig_y * width..(orig_y + 1) * width];
        for x in 0..width {
            if row[x] & 1 == 1 {
                let idx = rendered_y * width + x;
                let byte = idx >> 3;
                let shift = 7 - (idx & 7);
                out[byte] |= 1 << shift;
            }
        }
    }

    out
}

fn append_manifest(
    dir: &Path,
    input: &ExportInput,
    timestamp: &str,
    filename: &str,
) -> io::Result<()> {
    let path = dir.join(MANIFEST_FILE);
    let exists = path.exists();
    let mut f = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)?;

    if !exists {
        writeln!(
            f,
            "id\trule\twidth\tgenerations\tboundary\tstatus\tprogress\ttimestamp\tfilename"
        )?;
    }

    writeln!(
        f,
        "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
        input.job_id,
        input.rule,
        input.width,
        input.generations,
        input.boundary,
        input.status,
        input.progress,
        timestamp,
        filename,
    )?;

    Ok(())
}

struct ManifestEntry {
    id: String,
    rule: String,
    width: String,
    generations: String,
    boundary: String,
    status: String,
    progress: String,
    timestamp: String,
    filename: String,
}

fn read_manifest(dir: &Path) -> io::Result<Vec<ManifestEntry>> {
    let path = dir.join(MANIFEST_FILE);
    if !path.exists() {
        return Ok(Vec::new());
    }
    let f = File::open(&path)?;
    let reader = BufReader::new(f);
    let mut entries = Vec::new();
    for (i, line) in reader.lines().enumerate() {
        let line = line?;
        if i == 0 {
            continue;
        }
        if line.trim().is_empty() {
            continue;
        }
        let parts: Vec<&str> = line.split('\t').collect();
        if parts.len() < 9 {
            continue;
        }
        entries.push(ManifestEntry {
            id: parts[0].to_string(),
            rule: parts[1].to_string(),
            width: parts[2].to_string(),
            generations: parts[3].to_string(),
            boundary: parts[4].to_string(),
            status: parts[5].to_string(),
            progress: parts[6].to_string(),
            timestamp: parts[7].to_string(),
            filename: parts[8].to_string(),
        });
    }
    Ok(entries)
}

fn regenerate_index(dir: &Path) -> io::Result<()> {
    let entries = read_manifest(dir)?;

    let mut rows_html = String::new();
    for e in entries.iter().rev() {
        rows_html.push_str(&format!(
            "<tr data-filename=\"{}\">\
                <td>{}</td>\
                <td>{}</td>\
                <td>{}</td>\
                <td>{}/{}</td>\
                <td>{}</td>\
                <td>{}</td>\
                <td>{}</td>\
                <td><a href=\"{}\">open</a> | <button data-move>Move</button></td>\
             </tr>\n",
            html_escape(&e.filename),
            html_escape(&e.id),
            html_escape(&e.rule),
            html_escape(&e.width),
            html_escape(&e.progress),
            html_escape(&e.generations),
            html_escape(&e.boundary),
            html_escape(&e.status),
            html_escape(&e.timestamp),
            html_escape(&e.filename),
        ));
    }

    let count = entries.len();
    let html = format!(
        "<!doctype html><html lang=\"en\"><head>\n\
         <meta charset=\"utf-8\">\n\
         <title>Cellular Automata runs</title>\n\
         <style>\n\
         body {{ font-family: system-ui, sans-serif; margin: 20px; background: #fafafa; color: #222; }}\n\
         h1 {{ font-size: 20px; margin: 0 0 10px 0; }}\n\
         .meta {{ color: #666; font-size: 13px; margin-bottom: 12px; }}\n\
         #q {{ width: 100%; max-width: 520px; padding: 8px 10px; font-size: 14px; box-sizing: border-box; }}\n\
         table {{ width: 100%; border-collapse: collapse; margin-top: 12px; background: white; }}\n\
         th, td {{ text-align: left; padding: 6px 10px; border-bottom: 1px solid #eee; font-size: 13px; }}\n\
         th {{ background: #f0f0f0; position: sticky; top: 0; }}\n\
         tr:hover td {{ background: #fafaff; }}\n\
         a {{ color: #06c; text-decoration: none; }}\n\
         a:hover {{ text-decoration: underline; }}\n\
         .empty {{ color: #888; padding: 20px 0; }}\n\
         </style>\n\
         </head><body>\n\
         <h1>Cellular Automata runs</h1>\n\
         <div class=\"meta\">{} saved run(s). Type to filter by any column (rule, width, status, ...).</div>\n\
         <input id=\"q\" placeholder=\"Filter (e.g. rule 30 running)\" autofocus>\n\
         {}\n\
         <script>\n\
         let srcDirHandle = null;\n\
         let dstDirHandle = null;\n\
\n\
         (function(){{\n\
           const q = document.getElementById('q');\n\
           const rows = Array.from(document.querySelectorAll('#runs tbody tr'));\n\
           function apply() {{\n\
             const terms = q.value.toLowerCase().split(/\\s+/).filter(Boolean);\n\
             for (const r of rows) {{\n\
               const t = r.textContent.toLowerCase();\n\
               r.style.display = terms.every(w => t.includes(w)) ? '' : 'none';\n\
             }}\n\
           }}\n\
           q.addEventListener('input', apply);\n\
         }})();\n\
\n\
         async function ensureDirs() {{\n\
           if (!window.showDirectoryPicker) {{\n\
             alert('Move requires a Chromium browser with File System Access API.');\n\
             throw new Error('no FSA');\n\
           }}\n\
           if (!srcDirHandle) {{\n\
             srcDirHandle = await window.showDirectoryPicker({{mode: 'readwrite'}});\n\
           }}\n\
           if (!dstDirHandle) {{\n\
             dstDirHandle = await window.showDirectoryPicker({{mode: 'readwrite'}});\n\
           }}\n\
         }}\n\
\n\
         async function moveFile(btn) {{\n\
           try {{\n\
             await ensureDirs();\n\
             const tr = btn.closest('tr');\n\
             const fname = tr?.dataset?.filename;\n\
             if (!fname) {{ alert('Missing filename'); return; }}\n\
             const srcFile = await srcDirHandle.getFileHandle(fname);\n\
             const file = await srcFile.getFile();\n\
             const dstFile = await dstDirHandle.getFileHandle(fname, {{create: true}});\n\
             const writable = await dstFile.createWritable();\n\
             await writable.write(await file.arrayBuffer());\n\
             await writable.close();\n\
             try {{ await srcDirHandle.removeEntry(fname); }} catch(e) {{ console.warn('delete failed', e); }}\n\
             tr.remove();\n\
           }} catch (e) {{\n\
             console.error(e);\n\
             alert('Move failed: ' + e);\n\
           }}\n\
         }}\n\
\n\
         document.addEventListener('click', (ev) => {{\n\
           const target = ev.target;\n\
           if (target instanceof HTMLButtonElement && target.dataset.move !== undefined) {{\n\
             moveFile(target);\n\
           }}\n\
         }});\n\
         </script>\n\
         </body></html>\n",
        count,
        if entries.is_empty() {
            "<div class=\"empty\">No runs exported yet.</div>".to_string()
        } else {
            format!(
                "<table id=\"runs\">\n\
                 <thead><tr>\
                   <th>ID</th>\
                   <th>Rule</th>\
                   <th>Width</th>\
                   <th>Progress</th>\
                   <th>Boundary</th>\
                   <th>Status</th>\
                   <th>Exported</th>\
                   <th></th>\
                 </tr></thead>\n\
                 <tbody>\n{}\
                 </tbody>\n\
                 </table>",
                rows_html
            )
        }
    );

    fs::write(dir.join(INDEX_FILE), html)?;
    Ok(())
}

fn html_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

fn runs_dir() -> io::Result<PathBuf> {
    // Save exports into the user-requested data folder.
    Ok(PathBuf::from(DATA_DIR))
}
