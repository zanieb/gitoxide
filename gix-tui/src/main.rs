#![forbid(unsafe_code)]

use std::process::ExitCode;

fn main() -> ExitCode {
    eprintln!("gixi: the terminal UI is not implemented yet; use `gix` or `ein` for command-line workflows.");
    ExitCode::from(2)
}
