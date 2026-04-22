#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoundaryMode {
    ZeroPadded,
    Wrap,
}

impl BoundaryMode {
    pub const ALL: [BoundaryMode; 2] = [BoundaryMode::ZeroPadded, BoundaryMode::Wrap];
}

impl std::fmt::Display for BoundaryMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BoundaryMode::ZeroPadded => write!(f, "Padded"),
            BoundaryMode::Wrap => write!(f, "Wrap-around"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaddingAlign {
    After,
    Before,
    Center,
}

impl PaddingAlign {
    pub const ALL: [PaddingAlign; 3] = [PaddingAlign::After, PaddingAlign::Before, PaddingAlign::Center];
}

impl std::fmt::Display for PaddingAlign {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PaddingAlign::After => write!(f, "Padding after"),
            PaddingAlign::Before => write!(f, "Padding before"),
            PaddingAlign::Center => write!(f, "Centered"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaddingFill {
    Zero,
    One,
}

impl PaddingFill {
    pub const ALL: [PaddingFill; 2] = [PaddingFill::Zero, PaddingFill::One];

    pub fn value(self) -> u8 {
        match self {
            PaddingFill::Zero => 0,
            PaddingFill::One => 1,
        }
    }
}

impl std::fmt::Display for PaddingFill {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PaddingFill::Zero => write!(f, "Fill 0s"),
            PaddingFill::One => write!(f, "Fill 1s"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Rule {
    #[allow(dead_code)]
    pub number: u8,
    table: [u8; 8],
}

impl Rule {
    pub fn new(number: u8) -> Self {
        let mut table = [0u8; 8];
        for pattern in 0u8..8 {
            table[pattern as usize] = (number >> pattern) & 1;
        }
        Rule { number, table }
    }

    #[inline]
    pub fn apply(&self, left: u8, center: u8, right: u8) -> u8 {
        let idx = ((left & 1) << 2) | ((center & 1) << 1) | (right & 1);
        self.table[idx as usize]
    }

    #[allow(dead_code)]
    pub fn table(&self) -> &[u8; 8] {
        &self.table
    }
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct CellularAutomaton {
    pub rule: Rule,
    pub width: usize,
    pub generations: usize,
    pub initial: Vec<u8>,
    pub boundary: BoundaryMode,
    rows: Vec<Vec<u8>>,
}

#[allow(dead_code)]
impl CellularAutomaton {
    pub fn new(
        rule: Rule,
        width: usize,
        generations: usize,
        initial: Vec<u8>,
        boundary: BoundaryMode,
    ) -> Self {
        CellularAutomaton {
            rule,
            width,
            generations,
            initial,
            boundary,
            rows: Vec::new(),
        }
    }

    pub fn rows(&self) -> &[Vec<u8>] {
        &self.rows
    }

    pub fn run(&mut self) {
        let width = self.width;
        if width == 0 {
            self.rows = Vec::new();
            return;
        }

        let row0 = self.initial.clone();
        let total_rows = self.generations + 1;
        let mut rows: Vec<Vec<u8>> = Vec::with_capacity(total_rows);
        rows.push(row0);

        for g in 0..self.generations {
            let next = step_row(&self.rule, &rows[g], self.boundary, 0);
            rows.push(next);
        }

        self.rows = rows;
    }
}

pub fn make_initial_row(width: usize, seed: &[u8], align: PaddingAlign, fill: u8) -> Vec<u8> {
    let mut row = vec![fill & 1; width];
    if width == 0 || seed.is_empty() {
        return row;
    }
    let len = seed.len().min(width);
    let start = match align {
        PaddingAlign::Before => width - len,
        PaddingAlign::After => 0,
        PaddingAlign::Center => (width.saturating_sub(len)) / 2,
    };
    for (i, v) in seed.iter().take(len).enumerate() {
        row[start + i] = if *v != 0 { 1 } else { 0 };
    }
    row
}

/// Write the next generation row into `next`. `prev` and `next` must have the
/// same length; this function does not allocate and does not use threads.
///
/// The previous per-row `rayon::into_par_iter` version was disastrously
/// expensive for long runs: with width ~1000 and millions of generations, the
/// thread-dispatch overhead per row dwarfed the actual compute and prevented
/// any other thread (including the UI) from getting CPU time.
pub fn step_row_into(rule: &Rule, prev: &[u8], next: &mut [u8], boundary: BoundaryMode, boundary_fill: u8) {
    let width = prev.len();
    debug_assert_eq!(width, next.len());
    if width == 0 {
        return;
    }
    let fill = boundary_fill & 1;
    match boundary {
        BoundaryMode::ZeroPadded => {
            for i in 0..width {
                let l = if i == 0 { fill } else { prev[i - 1] };
                let r = if i + 1 >= width { fill } else { prev[i + 1] };
                next[i] = rule.apply(l, prev[i], r);
            }
        }
        BoundaryMode::Wrap => {
            if width == 1 {
                let c = prev[0];
                next[0] = rule.apply(c, c, c);
                return;
            }
            next[0] = rule.apply(prev[width - 1], prev[0], prev[1]);
            for i in 1..width - 1 {
                next[i] = rule.apply(prev[i - 1], prev[i], prev[i + 1]);
            }
            next[width - 1] = rule.apply(prev[width - 2], prev[width - 1], prev[0]);
        }
    }
}

pub fn step_row(rule: &Rule, prev: &[u8], boundary: BoundaryMode, boundary_fill: u8) -> Vec<u8> {
    let mut next = vec![0u8; prev.len()];
    step_row_into(rule, prev, &mut next, boundary, boundary_fill);
    next
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rule_30_table() {
        let r = Rule::new(30);
        assert_eq!(r.apply(1, 1, 1), 0);
        assert_eq!(r.apply(1, 1, 0), 0);
        assert_eq!(r.apply(1, 0, 1), 0);
        assert_eq!(r.apply(1, 0, 0), 1);
        assert_eq!(r.apply(0, 1, 1), 1);
        assert_eq!(r.apply(0, 1, 0), 1);
        assert_eq!(r.apply(0, 0, 1), 1);
        assert_eq!(r.apply(0, 0, 0), 0);
    }

    /// Headless smoke benchmark for the sequential simulation path.
    /// Runs the same inner loop the GUI worker uses, just with timing. Handy
    /// for verifying the right binary is being built and how fast rule 30
    /// really goes at width 1000. Invoke with:
    ///   cargo test --release sim::tests::bench_2m_x_1000 -- --nocapture --ignored
    #[test]
    #[ignore]
    fn bench_2m_x_1000() {
        let width: usize = 1000;
        let generations: usize = 2_000_000;
        let rule = Rule::new(30);

        let mut row0 = vec![0u8; width];
        row0[width / 2] = 1;

        let total_cells = (generations + 1) * width;
        let mut flat: Vec<u8> = Vec::with_capacity(total_cells);
        flat.extend_from_slice(&row0);

        let mut prev = row0;
        let mut next = vec![0u8; width];

        let start = std::time::Instant::now();
        for _ in 0..generations {
            step_row_into(&rule, &prev, &mut next, BoundaryMode::Wrap, 0);
            flat.extend_from_slice(&next);
            std::mem::swap(&mut prev, &mut next);
        }
        let elapsed = start.elapsed();

        let gens_per_sec = generations as f64 / elapsed.as_secs_f64();
        eprintln!(
            "bench_2m_x_1000: {} gens x {} width in {:.2?} ({:.0} gens/s, {:.1} Mcells/s, flat={} MiB)",
            generations,
            width,
            elapsed,
            gens_per_sec,
            (generations as f64 * width as f64) / elapsed.as_secs_f64() / 1e6,
            flat.len() / (1024 * 1024),
        );
        assert_eq!(flat.len(), total_cells);
    }

    #[test]
    fn rows_have_fixed_width() {
        let mut ca = CellularAutomaton::new(
            Rule::new(30),
            21,
            5,
            {
                let mut v = vec![0u8; 21];
                v[10] = 1;
                v
            },
            BoundaryMode::ZeroPadded,
        );
        ca.run();
        assert_eq!(ca.rows().len(), 6);
        for row in ca.rows() {
            assert_eq!(row.len(), 21);
        }
    }
}
