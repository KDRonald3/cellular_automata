//! Thin binary entry point. Builds the default `SimConfig` /
//! `InitialRow` / `RenderOptions` the UI used to start with, then hands off
//! to `cellular_automata::run(..., OutputKind::Ui)`. All real behaviour
//! lives in the library.

use std::process::ExitCode;

use cellular_automata::{
    run, BoundaryMode, InitialRow, OutputKind, PaddingAlign, PaddingFill, RenderOptions, SimConfig,
};

const DEFAULT_INITIAL: &str = "0000000000000000000001000000000000000000000";
const DEFAULT_RULE: u8 = 30;
const DEFAULT_GENERATIONS: usize = 200;
const DEFAULT_WIDTH: usize = 401;
const DEFAULT_CELL_SIZE: u32 = 4;

fn main() -> ExitCode {
    let seed: Vec<u8> = DEFAULT_INITIAL
        .chars()
        .filter_map(|c| c.to_digit(10).map(|d| if d == 0 { 0u8 } else { 1u8 }))
        .collect();

    let config = SimConfig {
        rule: DEFAULT_RULE,
        width: DEFAULT_WIDTH,
        generations: DEFAULT_GENERATIONS,
        boundary: BoundaryMode::ZeroPadded,
    };
    let initial = InitialRow::FromSeed {
        seed,
        align: PaddingAlign::Center,
        fill: PaddingFill::Zero,
    };
    let render = RenderOptions {
        cell_size: DEFAULT_CELL_SIZE,
        show_borders: true,
    };

    match run(config, initial, render, OutputKind::Ui) {
        Ok(_) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}
