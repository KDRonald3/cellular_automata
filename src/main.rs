mod export;
mod sim;

use std::cell::RefCell;
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use iced::mouse;
use iced::widget::canvas::{self, Cache, Geometry, Path, Stroke};
use iced::widget::{
    button, column, container, pick_list, responsive, row, scrollable, text, text_input,
    Canvas, Space,
};
use iced::{Color, Element, Fill, Length, Rectangle, Renderer, Size, Theme};

use sim::{make_initial_row, step_row_into, BoundaryMode, PaddingAlign, PaddingFill, Rule};

const DEFAULT_INITIAL: &str = "0000000000000000000001000000000000000000000";
const DEFAULT_RULE: u16 = 30;
const DEFAULT_GENERATIONS: usize = 200;
const DEFAULT_WIDTH: usize = 401;
const DEFAULT_CELL_SIZE: f32 = 4.0;
// Keep canvas dimensions well under typical `max_texture_dimension_2d` (8192/16384).
const MAX_CANVAS_SIDE_PX: f32 = 8_000.0;
// Per-tile budget on number of cells tessellated into a single canvas cache.
// Empirically keeps the cache's geometry buffer comfortably below wgpu's
// 256 MB single-buffer limit for typical rules and run lengths.
const TILE_CELL_BUDGET: usize = 2_000_000;
// Hard cap on how many vertical tiles the finished-canvas view may emit.
// Beyond this iced's layout work and potential cached-geometry footprint
// dominates, so we fall back to stride-based row downsampling and tell the
// user to use the HTML export for true full fidelity.
const MAX_TILES: usize = 64;
const MAX_GENERATIONS: u128 = 1_000_000_000;
// Hard cap on the total number of cells a single run may occupy in RAM.
// The flat backing buffer is one byte per cell, so this is the upper bound
// on the worker's working-set in bytes (currently 8 GiB). Runs that exceed
// this are rejected up front with a helpful message.
const MAX_TOTAL_CELLS: u128 = 8 * 1024 * 1024 * 1024;

// Bump this whenever the worker or rendering path changes materially. It's
// shown in the window title so you can visually confirm the running binary
// matches your current source tree (useful on Windows where cargo sometimes
// fails to replace a running .exe).
const BUILD_TAG: &str = "flat-buffer v3";

pub fn main() -> iced::Result {
    iced::application(App::title, App::update, App::view)
        .subscription(App::subscription)
        .theme(|_| Theme::Light)
        .run_with(App::new)
}

type JobId = u64;

#[derive(Clone, Debug)]
struct JobParams {
    rule_number: u8,
    width: usize,
    generations: usize,
    initial: Vec<u8>,
    boundary: BoundaryMode,
    padding_fill: PaddingFill,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum JobStatus {
    Running,
    Done,
    Cancelled,
}

impl std::fmt::Display for JobStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            JobStatus::Running => write!(f, "Running"),
            JobStatus::Done => write!(f, "Done"),
            JobStatus::Cancelled => write!(f, "Cancelled"),
        }
    }
}

struct JobShared {
    // Row-major flat grid: cell (y, x) = rows[y * width + x]. Stored as a
    // single contiguous allocation so long runs don't thrash the allocator
    // with millions of tiny per-row Vec<u8> objects.
    rows: Vec<u8>,
    width: usize,
    status: JobStatus,
    progress: usize,
}

impl JobShared {
    fn row_count(&self) -> usize {
        if self.width == 0 {
            0
        } else {
            self.rows.len() / self.width
        }
    }
}

struct Job {
    id: JobId,
    params: JobParams,
    shared: Arc<Mutex<JobShared>>,
    cancel: Arc<AtomicBool>,
    _handle: Option<std::thread::JoinHandle<()>>,
    // One Cache per vertical tile of the finished canvas. Grown lazily during
    // view construction; cleared whenever the job status changes or the tab is
    // re-selected.
    tile_caches: RefCell<Vec<Cache>>,
    last_seen_status: JobStatus,
    ewma_rate: Option<f64>,
    last_rate_sample: Option<(Instant, usize)>,
}

#[derive(Debug, Clone)]
enum Message {
    InitialChanged(String),
    RuleChanged(String),
    GenerationsChanged(String),
    WidthChanged(String),
    CellSizeChanged(String),
    BoundaryChanged(BoundaryMode),
    PaddingAlignChanged(PaddingAlign),
    PaddingFillChanged(PaddingFill),
    Run,
    SelectJob(JobId),
    CancelJob(JobId),
    RemoveJob(JobId),
    ExportJob(JobId),
    Tick,
}

struct App {
    initial_text: String,
    rule_text: String,
    generations_text: String,
    width_text: String,
    cell_size_text: String,
    boundary: BoundaryMode,
    padding_align: PaddingAlign,
    padding_fill: PaddingFill,
    next_job_id: JobId,
    jobs: Vec<Job>,
    selected: Option<JobId>,
    error: Option<String>,
}

impl App {
    fn title(&self) -> String {
        format!("Cellular Automaton [{}]", BUILD_TAG)
    }

    fn new() -> (Self, iced::Task<Message>) {
        (
            App {
                initial_text: DEFAULT_INITIAL.to_string(),
                rule_text: DEFAULT_RULE.to_string(),
                generations_text: DEFAULT_GENERATIONS.to_string(),
                width_text: DEFAULT_WIDTH.to_string(),
                cell_size_text: format!("{}", DEFAULT_CELL_SIZE as i32),
                boundary: BoundaryMode::ZeroPadded,
                padding_align: PaddingAlign::Center,
                padding_fill: PaddingFill::Zero,
                next_job_id: 1,
                jobs: Vec::new(),
                selected: None,
                error: None,
            },
            iced::Task::none(),
        )
    }

    fn update(&mut self, message: Message) {
        match message {
            Message::InitialChanged(s) => self.initial_text = s,
            Message::RuleChanged(s) => self.rule_text = s,
            Message::GenerationsChanged(s) => self.generations_text = s,
            Message::WidthChanged(s) => self.width_text = s,
            Message::CellSizeChanged(s) => self.cell_size_text = s,
            Message::BoundaryChanged(b) => self.boundary = b,
            Message::PaddingAlignChanged(a) => self.padding_align = a,
            Message::PaddingFillChanged(f) => self.padding_fill = f,
            Message::Run => {
                self.error = None;
                match self.parse_params() {
                    Ok(params) => {
                        let id = self.next_job_id;
                        self.next_job_id += 1;
                        let shared = Arc::new(Mutex::new(JobShared {
                            rows: Vec::new(),
                            width: params.width,
                            status: JobStatus::Running,
                            progress: 0,
                        }));
                        let cancel = Arc::new(AtomicBool::new(false));
                        let handle =
                            spawn_worker(params.clone(), shared.clone(), cancel.clone());
                        self.jobs.push(Job {
                            id,
                            params,
                            shared,
                            cancel,
                            _handle: Some(handle),
                            tile_caches: RefCell::new(Vec::new()),
                            last_seen_status: JobStatus::Running,
                            ewma_rate: None,
                            last_rate_sample: None,
                        });
                        self.selected = Some(id);
                    }
                    Err(e) => self.error = Some(e),
                }
            }
            Message::SelectJob(id) => {
                if self.jobs.iter().any(|j| j.id == id) {
                    self.selected = Some(id);
                    if let Some(job) = self.jobs.iter_mut().find(|j| j.id == id) {
                        clear_tile_caches(job);
                    }
                }
            }
            Message::CancelJob(id) => {
                if let Some(job) = self.jobs.iter().find(|j| j.id == id) {
                    job.cancel.store(true, Ordering::Relaxed);
                }
            }
            Message::RemoveJob(id) => {
                if let Some(pos) = self.jobs.iter().position(|j| j.id == id) {
                    self.jobs[pos].cancel.store(true, Ordering::Relaxed);
                    self.jobs.remove(pos);
                    if self.selected == Some(id) {
                        self.selected = self.jobs.first().map(|j| j.id);
                    }
                }
            }
            Message::ExportJob(id) => {
                if let Some(job) = self.jobs.iter().find(|j| j.id == id) {
                    let (rows, status, progress) = {
                        let s = job.shared.lock().unwrap();
                        // `rows` is a single contiguous Vec<u8>, so this clone
                        // is one allocation + memcpy regardless of run size.
                        (s.rows.clone(), s.status.clone(), s.progress)
                    };

                    let status_str = status.to_string();
                    let input = export::ExportInput {
                        job_id: job.id,
                        rule: job.params.rule_number,
                        width: job.params.width,
                        generations: job.params.generations,
                        progress,
                        boundary: job.params.boundary,
                        status: &status_str,
                        rows: &rows,
                    };
                    match export::export_job(&input) {
                        Ok(path) => {
                            let open_result = open_in_browser(&path);
                            match open_result {
                                Ok(()) => {
                                    self.error = Some(format!("Saved {} (opened)", path.display()))
                                }
                                Err(e) => {
                                    self.error = Some(format!(
                                        "Saved {} (open failed: {})",
                                        path.display(),
                                        e
                                    ))
                                }
                            }
                        }
                        Err(e) => {
                            self.error = Some(format!("Export failed: {e}"));
                        }
                    }
                }
            }
            Message::Tick => {
                for job in self.jobs.iter_mut() {
                    let (progress, status) = {
                        let s = job.shared.lock().unwrap();
                        (s.progress, s.status.clone())
                    };

                    let now = Instant::now();
                    let status_changed = status != job.last_seen_status;

                    match status {
                        JobStatus::Running => {
                            if let Some((prev_t, prev_p)) = job.last_rate_sample {
                                let dt = now.duration_since(prev_t).as_secs_f64();
                                let dp = progress.saturating_sub(prev_p);
                                if dt >= 0.15 && dp > 0 {
                                    let instant = dp as f64 / dt;
                                    job.ewma_rate = Some(match job.ewma_rate {
                                        Some(prev) => 0.8 * prev + 0.2 * instant,
                                        None => instant,
                                    });
                                    job.last_rate_sample = Some((now, progress));
                                }
                            } else {
                                job.last_rate_sample = Some((now, progress));
                            }
                        }
                        JobStatus::Done | JobStatus::Cancelled => {
                            job.last_rate_sample = None;
                        }
                    }

                    if status_changed {
                        clear_tile_caches(job);
                    }
                    job.last_seen_status = status;
                }
            }
        }
    }

    fn parse_params(&self) -> Result<JobParams, String> {
        let rule_n: u16 = self
            .rule_text
            .trim()
            .parse()
            .map_err(|_| "Rule must be a number 0-255".to_string())?;
        if rule_n > 255 {
            return Err("Rule must be 0-255".to_string());
        }
        let generations_raw: u128 = self
            .generations_text
            .trim()
            .parse()
            .map_err(|_| format!("Generations must be a number between 0 and {}", MAX_GENERATIONS))?;
        if generations_raw > MAX_GENERATIONS {
            return Err(format!(
                "Generations too large. Max supported is {}.",
                MAX_GENERATIONS
            ));
        }
        let generations: usize = generations_raw
            .try_into()
            .map_err(|_| "Generations value is too large for this platform".to_string())?;
        let width: usize = self
            .width_text
            .trim()
            .parse()
            .map_err(|_| "Width must be a positive integer".to_string())?;
        if width == 0 {
            return Err("Width must be greater than 0".to_string());
        }

        let total_cells = (generations_raw + 1).saturating_mul(width as u128);
        if total_cells > MAX_TOTAL_CELLS {
            return Err(format!(
                "Run is too large: {} cells (~{:.1} GiB) exceeds the {} GiB in-memory limit. \
                 Reduce generations or width.",
                total_cells,
                total_cells as f64 / (1024.0 * 1024.0 * 1024.0),
                MAX_TOTAL_CELLS / (1024 * 1024 * 1024),
            ));
        }

        let parsed: Vec<u8> = self
            .initial_text
            .chars()
            .filter_map(|c| c.to_digit(10).map(|d| if d == 0 { 0u8 } else { 1u8 }))
            .collect();
        let initial = make_initial_row(width, &parsed, self.padding_align, self.padding_fill.value());

        Ok(JobParams {
            rule_number: rule_n as u8,
            width,
            generations,
            initial,
            boundary: self.boundary,
            padding_fill: self.padding_fill,
        })
    }

    fn cell_size(&self) -> f32 {
        self.cell_size_text
            .trim()
            .parse::<f32>()
            .ok()
            .filter(|v| *v > 0.0)
            .unwrap_or(DEFAULT_CELL_SIZE)
    }

    fn subscription(&self) -> iced::Subscription<Message> {
        let any_running = self
            .jobs
            .iter()
            .any(|j| j.shared.lock().map(|s| s.status == JobStatus::Running).unwrap_or(false));
        if any_running {
            // 250ms ticks are plenty to drive ETA updates; no canvas work happens
            // while running so we don't need a high-frequency refresh.
            iced::time::every(Duration::from_millis(250)).map(|_| Message::Tick)
        } else {
            iced::Subscription::none()
        }
    }

    fn view(&self) -> Element<'_, Message> {
        let controls = self.controls_view();

        let status_line: Option<Element<'_, Message>> = self.error.as_ref().map(|msg| {
            let is_error = msg.to_lowercase().contains("fail");
            let color = if is_error {
                Color::from_rgb(0.8, 0.0, 0.0)
            } else {
                Color::from_rgb(0.0, 0.45, 0.0)
            };
            text(msg).color(color).into()
        });

        let tabs = self.tabs_view();

        let body: Element<'_, Message> =
            match self.selected.and_then(|id| self.jobs.iter().find(|j| j.id == id)) {
                Some(job) => self.detail_view(job),
                None => container(
                    text("No jobs yet. Configure the parameters above and press Run.")
                        .color(Color::from_rgb(0.4, 0.4, 0.4)),
                )
                .center_x(Fill)
                .center_y(Fill)
                .width(Fill)
                .height(Fill)
                .into(),
            };

        let mut col = column![controls].spacing(10).padding(10);
        if let Some(status) = status_line {
            col = col.push(status);
        }
        col = col.push(tabs).push(body);

        container(col.width(Fill).height(Fill))
            .width(Fill)
            .height(Fill)
            .into()
    }

    fn controls_view(&self) -> Element<'_, Message> {
        let initial_input = text_input("Initial state (e.g. 0001000)", &self.initial_text)
            .on_input(Message::InitialChanged)
            .padding(6)
            .width(Length::Fixed(280.0));

        let rule_input = text_input("30", &self.rule_text)
            .on_input(Message::RuleChanged)
            .padding(6)
            .width(Length::Fixed(80.0));

        let gens_input = text_input("200", &self.generations_text)
            .on_input(Message::GenerationsChanged)
            .padding(6)
            .width(Length::Fixed(90.0));

        let width_input = text_input("401", &self.width_text)
            .on_input(Message::WidthChanged)
            .padding(6)
            .width(Length::Fixed(90.0));

        let cell_size_input = text_input("4", &self.cell_size_text)
            .on_input(Message::CellSizeChanged)
            .padding(6)
            .width(Length::Fixed(70.0));

        let boundary_pick = pick_list(
            &BoundaryMode::ALL[..],
            Some(self.boundary),
            Message::BoundaryChanged,
        )
        .width(Length::Fixed(160.0));

        let padding_pick = pick_list(
            &PaddingAlign::ALL[..],
            Some(self.padding_align),
            Message::PaddingAlignChanged,
        )
        .width(Length::Fixed(160.0));

        let fill_pick = pick_list(
            &PaddingFill::ALL[..],
            Some(self.padding_fill),
            Message::PaddingFillChanged,
        )
        .width(Length::Fixed(110.0));

        let run_button = button(text("Run")).on_press(Message::Run).padding(8);

        row![
            labeled("Initial", initial_input),
            labeled("Rule (0-255)", rule_input),
            labeled("Generations", gens_input),
            labeled("Width", width_input),
            labeled("Cell px", cell_size_input),
            labeled("Boundary", boundary_pick),
            labeled("Padding", padding_pick),
            labeled("Fill", fill_pick),
            run_button,
        ]
        .spacing(12)
        .align_y(iced::Alignment::End)
        .wrap()
        .into()
    }

    fn tabs_view(&self) -> Element<'_, Message> {
        if self.jobs.is_empty() {
            return Element::from(text(""));
        }

        let mut tabs_row = row![].spacing(6);
        for job in &self.jobs {
            let is_active = self.selected == Some(job.id);

            let status = job.shared.lock().unwrap().status.clone();
            let status_marker = match status {
                JobStatus::Running => "...",
                JobStatus::Done => "",
                JobStatus::Cancelled => "x",
            };

            let label_text = format!(
                "#{} rule {}{}",
                job.id,
                job.params.rule_number,
                if status_marker.is_empty() {
                    String::new()
                } else {
                    format!(" [{}]", status_marker)
                },
            );

            let select_btn = {
                let b = button(text(label_text).size(13))
                    .on_press(Message::SelectJob(job.id))
                    .padding([4, 10]);
                if is_active {
                    b.style(button::primary)
                } else {
                    b.style(button::secondary)
                }
            };

            let close_btn = button(text("x").size(12))
                .on_press(Message::RemoveJob(job.id))
                .padding([4, 8])
                .style(button::secondary);

            let tab_pair = row![select_btn, close_btn].spacing(2);
            tabs_row = tabs_row.push(tab_pair);
        }

        scrollable(container(tabs_row).padding([0, 2]))
            .direction(scrollable::Direction::Horizontal(
                scrollable::Scrollbar::default(),
            ))
            .width(Fill)
            .into()
    }

    fn detail_view<'a>(&'a self, job: &'a Job) -> Element<'a, Message> {
        let (status, progress, rows_len) = {
            let s = job.shared.lock().unwrap();
            (s.status.clone(), s.progress, s.row_count())
        };

        let mut header_row = row![text(format!(
            "#{}  rule {}  width {}  {} / {} gens  [{}]  {}",
            job.id,
            job.params.rule_number,
            job.params.width,
            progress,
            job.params.generations,
            job.params.boundary,
            status,
        ))
        .size(13),]
        .spacing(10)
        .align_y(iced::Alignment::Center);

        if status == JobStatus::Running {
            header_row = header_row.push(
                button(text("Cancel"))
                    .on_press(Message::CancelJob(job.id))
                    .padding(6),
            );

            let remaining_gens = job.params.generations.saturating_sub(progress);
            let pct = if job.params.generations == 0 {
                100.0
            } else {
                (progress as f64 / job.params.generations as f64) * 100.0
            };
            let eta_text = if remaining_gens == 0 {
                format!("{:.2}% - finalizing...", pct)
            } else if let Some(rate) = job.ewma_rate.filter(|r| *r > 0.0) {
                format!(
                    "{:.2}% - ETA ~ {}  ({})",
                    pct,
                    format_duration(remaining_gens as f64 / rate),
                    format_rate(rate),
                )
            } else {
                format!("{:.2}% - ETA ...", pct)
            };
            header_row = header_row.push(
                text(eta_text)
                    .size(12)
                    .color(Color::from_rgb(0.4, 0.4, 0.4)),
            );
        } else if status == JobStatus::Done {
            header_row = header_row.push(
                text("100.00%")
                    .size(12)
                    .color(Color::from_rgb(0.4, 0.4, 0.4)),
            );
        }
        header_row = header_row.push(
            button(text("Export HTML"))
                .on_press(Message::ExportJob(job.id))
                .padding(6),
        );
        header_row = header_row.push(
            button(text("Remove"))
                .on_press(Message::RemoveJob(job.id))
                .padding(6),
        );

        // While the job is running we do NOT render the canvas; only show a
        // progress placeholder. The canvas is only drawn when the job has
        // finished (or was cancelled) so we can render the full result in one
        // pass and keep the UI responsive during computation.
        let body: Element<'_, Message> = if status == JobStatus::Running {
            self.running_placeholder(job, progress)
        } else {
            self.finished_canvas(job, rows_len, status)
        };

        column![header_row, body]
            .spacing(8)
            .width(Fill)
            .height(Fill)
            .into()
    }

    fn running_placeholder<'a>(
        &'a self,
        job: &'a Job,
        progress: usize,
    ) -> Element<'a, Message> {
        let pct = if job.params.generations == 0 {
            100.0
        } else {
            (progress as f64 / job.params.generations as f64) * 100.0
        };

        let rate_line = match job.ewma_rate {
            Some(r) if r > 0.0 => format_rate(r),
            _ => "measuring rate...".to_string(),
        };

        let eta_line = match job.ewma_rate {
            Some(r) if r > 0.0 => {
                let remaining_gens = job.params.generations.saturating_sub(progress);
                format!("ETA ~ {}", format_duration(remaining_gens as f64 / r))
            }
            _ => "ETA ...".to_string(),
        };

        let content = column![
            text("Running simulation")
                .size(18)
                .color(Color::from_rgb(0.25, 0.25, 0.25)),
            Space::with_height(Length::Fixed(8.0)),
            text(format!(
                "{:.2}%  ({} / {} generations)",
                pct, progress, job.params.generations
            ))
            .size(14),
            text(rate_line)
                .size(12)
                .color(Color::from_rgb(0.4, 0.4, 0.4)),
            text(eta_line)
                .size(12)
                .color(Color::from_rgb(0.4, 0.4, 0.4)),
            Space::with_height(Length::Fixed(12.0)),
            text("The result will appear here when the run finishes.")
                .size(12)
                .color(Color::from_rgb(0.5, 0.5, 0.5)),
        ]
        .spacing(4)
        .align_x(iced::Alignment::Center);

        container(content)
            .center_x(Fill)
            .center_y(Fill)
            .width(Fill)
            .height(Fill)
            .into()
    }

    fn finished_canvas<'a>(
        &'a self,
        job: &'a Job,
        rows_len: usize,
        status: JobStatus,
    ) -> Element<'a, Message> {
        // Auto-shrink cell size so the canvas width stays within safe GPU
        // texture dimensions regardless of how wide the automaton is.
        let user_cell = self.cell_size();
        let cell = user_cell
            .min(MAX_CANVAS_SIDE_PX / (job.params.width.max(1) as f32))
            .max(0.25);
        let grid_w = (job.params.width as f32 * cell).max(1.0);

        // Attempt full-fidelity rendering: render every row at stride 1, but
        // split the image into vertical tiles so each tile's canvas texture
        // and geometry buffer stay within GPU limits. Tile height is bounded
        // by both the max safe canvas side in pixels AND by a per-tile cell
        // budget so that no single tile's cached geometry overflows wgpu's
        // buffer limits.
        let max_rows_by_pixels = ((MAX_CANVAS_SIDE_PX / cell).floor() as usize).max(1);
        let max_rows_by_cells = (TILE_CELL_BUDGET / job.params.width.max(1)).max(1);
        let rows_per_tile = max_rows_by_pixels.min(max_rows_by_cells).max(1);

        let effective_rows = rows_len.max(1);

        // If full fidelity would require more than MAX_TILES canvases, pick
        // the smallest stride that keeps the tile count under the cap. This
        // guarantees the widget tree stays small and the total cached
        // geometry stays bounded, at the cost of sampling rows.
        let natural_tiles = ((effective_rows + rows_per_tile - 1) / rows_per_tile).max(1);
        let (render_stride, render_rows_len, num_tiles) = if natural_tiles <= MAX_TILES {
            (1usize, effective_rows, natural_tiles)
        } else {
            let stride = ((natural_tiles + MAX_TILES - 1) / MAX_TILES).max(1);
            let rendered = (effective_rows + stride - 1) / stride;
            let tiles = ((rendered + rows_per_tile - 1) / rows_per_tile).max(1);
            (stride, rendered, tiles)
        };

        // Ensure we have one cache per tile. Existing caches are kept so
        // redraws (viewport resize, etc.) remain cheap.
        {
            let mut caches = job.tile_caches.borrow_mut();
            while caches.len() < num_tiles {
                caches.push(Cache::default());
            }
        }

        let total_grid_h = render_rows_len as f32 * cell;

        let mut info_col = column![].spacing(4);
        if status == JobStatus::Cancelled {
            info_col = info_col.push(
                text("Job was cancelled; partial result below.")
                    .size(12)
                    .color(Color::from_rgb(0.6, 0.3, 0.0)),
            );
        }
        if render_stride > 1 {
            info_col = info_col.push(
                text(format!(
                    "downsampled for display: stride {}x ({} of {} rows across {} tiles). \
                     Use Export HTML for full fidelity.",
                    render_stride, render_rows_len, rows_len, num_tiles
                ))
                .size(11)
                .color(Color::from_rgb(0.55, 0.35, 0.0)),
            );
        } else if num_tiles > 1 {
            info_col = info_col.push(
                text(format!(
                    "full-fidelity render: {} rows across {} tiles ({} rows/tile)",
                    rows_len, num_tiles, rows_per_tile
                ))
                .size(11)
                .color(Color::from_rgb(0.4, 0.4, 0.4)),
            );
        }

        let body = responsive(move |viewport| {
            let content_w = grid_w.max(viewport.width).max(1.0);
            let content_h = total_grid_h.max(viewport.height).max(1.0);

            let mut tile_col = column![].spacing(0.0);
            for tile_idx in 0..num_tiles {
                // Row indices here are into the downsampled / rendered row
                // space (i.e. after stride has been applied). The TileProgram
                // maps them back to original rows via render_stride.
                let rendered_start = tile_idx * rows_per_tile;
                let rendered_end = (rendered_start + rows_per_tile).min(render_rows_len);
                let tile_rows = rendered_end - rendered_start;
                let tile_h = (tile_rows as f32 * cell).max(1.0);
                let is_last = tile_idx + 1 == num_tiles;

                let prog = TileProgram {
                    job,
                    cell_size: cell,
                    rendered_row_start: rendered_start,
                    rendered_row_end: rendered_end,
                    render_stride,
                    tile_idx,
                    draw_top_border: tile_idx == 0,
                    draw_bottom_border: is_last,
                };
                let canvas: Element<'_, Message> = Canvas::new(prog)
                    .width(Length::Fixed(grid_w))
                    .height(Length::Fixed(tile_h))
                    .into();
                tile_col = tile_col.push(canvas);
            }

            let centered = container(tile_col)
                .width(Length::Fixed(content_w))
                .height(Length::Fixed(content_h))
                .center_x(Length::Fixed(content_w))
                .center_y(Length::Fixed(content_h))
                .style(|_t| container::Style {
                    background: Some(iced::Background::Color(Color::WHITE)),
                    ..container::Style::default()
                });

            scrollable(centered)
                .direction(scrollable::Direction::Both {
                    vertical: scrollable::Scrollbar::default(),
                    horizontal: scrollable::Scrollbar::default(),
                })
                .width(Fill)
                .height(Fill)
                .into()
        });

        // Outer white container covers the full viewport on every frame,
        // preventing the one-frame black flash that occurs on window expansion
        // while the inner `responsive` closure hasn't fired yet.
        let canvas_area = container(body)
            .width(Fill)
            .height(Fill)
            .style(|_t| container::Style {
                background: Some(iced::Background::Color(Color::WHITE)),
                ..container::Style::default()
            });

        column![info_col, canvas_area]
            .spacing(6)
            .width(Fill)
            .height(Fill)
            .into()
    }
}

fn clear_tile_caches(job: &mut Job) {
    for cache in job.tile_caches.borrow_mut().iter_mut() {
        cache.clear();
    }
}

fn open_in_browser(path: &std::path::Path) -> std::io::Result<()> {
    #[cfg(target_os = "windows")]
    {
        Command::new("cmd")
            .args(["/C", "start", "", path.to_str().unwrap_or("")])
            .spawn()
            .map(|_| ())
    }
    #[cfg(target_os = "macos")]
    {
        Command::new("open").arg(path).spawn().map(|_| ())
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        Command::new("xdg-open").arg(path).spawn().map(|_| ())
    }
}

fn labeled<'a>(
    label: &'a str,
    widget: impl Into<Element<'a, Message>>,
) -> Element<'a, Message> {
    column![text(label).size(12), widget.into()]
        .spacing(2)
        .into()
}

fn format_duration(secs: f64) -> String {
    if !secs.is_finite() || secs < 0.0 {
        return "...".to_string();
    }
    let total = secs.round() as u64;
    let days = total / 86_400;
    let hours = (total % 86_400) / 3600;
    let minutes = (total % 3600) / 60;
    let seconds = total % 60;
    if days > 0 {
        format!("{}d {:02}:{:02}:{:02}", days, hours, minutes, seconds)
    } else if hours > 0 {
        format!("{:02}:{:02}:{:02}", hours, minutes, seconds)
    } else {
        format!("{:02}:{:02}", minutes, seconds)
    }
}

fn format_rate(rate: f64) -> String {
    if !rate.is_finite() || rate <= 0.0 {
        return "... gens/s".to_string();
    }
    if rate >= 1000.0 {
        format!("{:.0} gens/s", rate)
    } else if rate >= 100.0 {
        format!("{:.1} gens/s", rate)
    } else {
        format!("{:.2} gens/s", rate)
    }
}

fn spawn_worker(
    params: JobParams,
    shared: Arc<Mutex<JobShared>>,
    cancel: Arc<AtomicBool>,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let rule = Rule::new(params.rule_number);
        let width = params.width;
        let row0 = params.initial.clone();

        // Single contiguous backing buffer for the whole history. One large
        // allocation replaces what used to be `generations + 1` separate
        // Vec<u8> objects, which on multi-million-generation runs caused
        // catastrophic allocator pressure and made the OS swap out the UI.
        let total_rows = params.generations.saturating_add(1);
        let total_cells = total_rows.saturating_mul(width);
        // Reserve the full backing buffer up front. If the OS refuses the
        // allocation we fall back to starting empty and growing on demand, so
        // the worker can still report progress / cancellation cleanly rather
        // than aborting the whole process.
        let mut flat: Vec<u8> = Vec::new();
        if flat.try_reserve_exact(total_cells).is_err() {
            let _ = flat.try_reserve(total_cells.min(4 * 1024 * 1024));
        }
        flat.extend_from_slice(&row0);

        // Two scratch buffers that swap each step: no per-generation allocation.
        let mut scratch_prev: Vec<u8> = row0;
        let mut scratch_next: Vec<u8> = vec![0u8; width];

        let progress_interval = Duration::from_millis(100);
        let mut last_progress_update = Instant::now();
        let mut cancelled = false;

        for g in 0..params.generations {
            if cancel.load(Ordering::Relaxed) {
                cancelled = true;
                break;
            }
            step_row_into(&rule, &scratch_prev, &mut scratch_next, params.boundary, params.padding_fill.value());
            flat.extend_from_slice(&scratch_next);
            std::mem::swap(&mut scratch_prev, &mut scratch_next);

            if last_progress_update.elapsed() >= progress_interval {
                let mut s = shared.lock().unwrap();
                s.progress = g + 1;
                drop(s);
                last_progress_update = Instant::now();
            }
        }

        // Publish the full history and final status in a single update so the
        // UI swaps from placeholder to canvas exactly once.
        let flat_len = flat.len();
        let mut s = shared.lock().unwrap();
        s.rows = flat;
        s.width = width;
        let row_count = if width == 0 { 0 } else { flat_len / width };
        if cancelled {
            s.progress = row_count.saturating_sub(1);
            s.status = JobStatus::Cancelled;
        } else {
            s.progress = params.generations;
            s.status = JobStatus::Done;
        }
    })
}

struct TileProgram<'a> {
    job: &'a Job,
    cell_size: f32,
    // Range in the *rendered* row space (post-stride). The original-row
    // index for rendered row r is `r * render_stride`.
    rendered_row_start: usize,
    rendered_row_end: usize,
    render_stride: usize,
    tile_idx: usize,
    draw_top_border: bool,
    draw_bottom_border: bool,
}

impl<'a> canvas::Program<Message> for TileProgram<'a> {
    type State = ();

    fn draw(
        &self,
        _state: &(),
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let caches = self.job.tile_caches.borrow();
        let cache = match caches.get(self.tile_idx) {
            Some(c) => c,
            None => return Vec::new(),
        };

        let geometry = cache.draw(renderer, bounds.size(), |frame| {
            frame.fill_rectangle(iced::Point::ORIGIN, bounds.size(), Color::WHITE);

            // Copy only this tile's rows out of the shared flat buffer. When
            // render_stride > 1 we sample one row out of every stride rows.
            // Cloning decouples us from the shared mutex so drawing can't
            // block the worker thread.
            let width = self.job.params.width;
            let stride = self.render_stride.max(1);
            let (tile_bytes, tile_rows) = {
                let s = self.job.shared.lock().unwrap();
                let total_rows = s.row_count();
                if width == 0 || self.rendered_row_start >= self.rendered_row_end {
                    (Vec::<u8>::new(), 0usize)
                } else if stride == 1 {
                    let start = self.rendered_row_start.min(total_rows);
                    let end = self.rendered_row_end.min(total_rows);
                    let bytes = s.rows[start * width..end * width].to_vec();
                    (bytes, end - start)
                } else {
                    let planned = self.rendered_row_end - self.rendered_row_start;
                    let mut bytes = Vec::with_capacity(planned * width);
                    let mut produced = 0usize;
                    for r in self.rendered_row_start..self.rendered_row_end {
                        let orig = r * stride;
                        if orig >= total_rows {
                            break;
                        }
                        bytes.extend_from_slice(&s.rows[orig * width..(orig + 1) * width]);
                        produced += 1;
                    }
                    (bytes, produced)
                }
            };
            if tile_rows == 0 {
                return;
            }

            let cell = self.cell_size.max(0.25);
            let grid_w = width as f32 * cell;
            let tile_h = tile_rows as f32 * cell;

            // Side borders are drawn on every tile so the outline is continuous;
            // top/bottom are only drawn on the first/last tile.
            let stroke = Stroke::default()
                .with_color(Color::from_rgb(0.85, 0.85, 0.85))
                .with_width(1.0);
            let left = Path::line(
                iced::Point::new(0.0, 0.0),
                iced::Point::new(0.0, tile_h),
            );
            let right = Path::line(
                iced::Point::new(grid_w, 0.0),
                iced::Point::new(grid_w, tile_h),
            );
            frame.stroke(&left, stroke.clone());
            frame.stroke(&right, stroke.clone());
            if self.draw_top_border {
                let top = Path::line(
                    iced::Point::new(0.0, 0.0),
                    iced::Point::new(grid_w, 0.0),
                );
                frame.stroke(&top, stroke.clone());
            }
            if self.draw_bottom_border {
                let bottom = Path::line(
                    iced::Point::new(0.0, tile_h),
                    iced::Point::new(grid_w, tile_h),
                );
                frame.stroke(&bottom, stroke.clone());
            }

            for row_idx in 0..tile_rows {
                let y = row_idx as f32 * cell;
                let row = &tile_bytes[row_idx * width..(row_idx + 1) * width];
                // Coalesce horizontal runs of live cells into a single fill.
                let mut x = 0usize;
                let n = row.len();
                while x < n {
                    if row[x] == 1 {
                        let start = x;
                        while x < n && row[x] == 1 {
                            x += 1;
                        }
                        let run_len = x - start;
                        frame.fill_rectangle(
                            iced::Point::new(start as f32 * cell, y),
                            Size::new(run_len as f32 * cell, cell),
                            Color::BLACK,
                        );
                    } else {
                        x += 1;
                    }
                }
            }
        });

        vec![geometry]
    }
}
