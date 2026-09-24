use std::{env, fs, process::ExitCode};

use pub_plccmob_projection::parse_confirmed_mature_plc_cmob;

fn run() -> Result<(), String> {
    let path = env::args()
        .nth(1)
        .ok_or_else(|| "usage: plccmob-wire-inspect <raw-chunk.bin>".to_owned())?;
    let raw = fs::read(&path).map_err(|error| format!("failed to read {path}: {error}"))?;
    let parsed = parse_confirmed_mature_plc_cmob(&raw)
        .map_err(|error| format!("PlcCmob wire rejected: {error}"))?;
    let json = serde_json::to_string_pretty(&parsed)
        .map_err(|error| format!("serialize parsed PlcCmob: {error}"))?;
    println!("{json}");
    Ok(())
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("plccmob-wire-inspect: {error}");
            ExitCode::FAILURE
        }
    }
}
