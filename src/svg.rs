//! Self-contained SVG generation for a simulation result.
//!
//! The SVG is laid out as one [`RenderOptions::cell_size`]-pixel square per
//! cell. To keep the file size tractable for long runs, consecutive black
//! cells in the same row are coalesced into a single `<rect>` (horizontal
//! run-length encoding). White cells are represented implicitly by the white
//! background, so a typical rule-30 output produces an order of magnitude
//! fewer rects than `width * generations`.
//!
//! Output is plain ASCII SVG with no embedded scripts or external references
//! — ready to drop into LaTeX, Inkscape, or a static web page.
//!
//! [`RenderOptions::cell_size`]: crate::RenderOptions::cell_size

use std::fmt::Write as _;

use crate::{RenderOptions, SimulationResult};

/// Above this many cells we print a one-line warning to stderr. The SVG is
/// still produced — the warning just nudges the caller toward
/// [`OutputKind::Structured`](crate::OutputKind::Structured) for very large
/// runs.
const LARGE_SVG_THRESHOLD: u128 = 50_000_000;

/// Convert a simulation result to a self-contained SVG document.
///
/// `opts.cell_size` controls the size of each cell in user-space units (1 =
/// 1 SVG pixel). `opts.show_borders` draws a light-grey grid between cells
/// when `cell_size > 1`; with `cell_size == 1` borders would overwhelm the
/// content so the flag is ignored.
pub fn to_svg(result: &SimulationResult, opts: &RenderOptions) -> String {
    let width = result.width;
    let height = if width == 0 { 0 } else { result.rows.len() / width };
    let cs = opts.cell_size.max(1) as usize;
    let total_w = width * cs;
    let total_h = height * cs;

    let cell_count = (width as u128) * (height as u128);
    if cell_count > LARGE_SVG_THRESHOLD {
        eprintln!(
            "warning: SVG output covers {} cells; many editors will struggle to open it. \
             Consider OutputKind::Structured for analysis at this scale.",
            cell_count
        );
    }

    let with_borders = opts.show_borders && cs > 1;
    let bg = if with_borders { "#4d4d4d" } else { "#ffffff" };

    let mut s = String::with_capacity(256 + cell_count.min(8_000_000) as usize / 4);
    let _ = writeln!(
        s,
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>"
    );
    let _ = writeln!(
        s,
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{}\" height=\"{}\" \
         viewBox=\"0 0 {} {}\" shape-rendering=\"crispEdges\">",
        total_w, total_h, total_w, total_h
    );
    let _ = writeln!(
        s,
        "<rect width=\"{}\" height=\"{}\" fill=\"{}\"/>",
        total_w, total_h, bg
    );

    if width == 0 || height == 0 {
        s.push_str("</svg>\n");
        return s;
    }

    if with_borders {
        // Each cell is inset by 1 unit on all sides; the dark background
        // shows through as the grid line. Matches the HTML export's
        // borders-on rendering exactly.
        let inset = (cs - 1) as i64;
        for y in 0..height {
            let row = &result.rows[y * width..(y + 1) * width];
            for x in 0..width {
                let color = if row[x] == 1 { "#000000" } else { "#ffffff" };
                let _ = writeln!(
                    s,
                    "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" fill=\"{}\"/>",
                    x * cs + 1,
                    y * cs + 1,
                    inset,
                    inset,
                    color
                );
            }
        }
    } else {
        // White background already painted; emit one <rect> per maximal
        // horizontal run of black cells. Default SVG fill is black, so we
        // can omit the `fill` attribute for an even smaller file.
        for y in 0..height {
            let row = &result.rows[y * width..(y + 1) * width];
            let mut x = 0;
            while x < width {
                if row[x] == 1 {
                    let start = x;
                    while x < width && row[x] == 1 {
                        x += 1;
                    }
                    let run_len = x - start;
                    let _ = writeln!(
                        s,
                        "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\"/>",
                        start * cs,
                        y * cs,
                        run_len * cs,
                        cs
                    );
                } else {
                    x += 1;
                }
            }
        }
    }

    s.push_str("</svg>\n");
    s
}
