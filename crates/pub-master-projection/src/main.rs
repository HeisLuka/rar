use std::{
    env, fs,
    io::{self, Read},
    process::ExitCode,
};

use pub_master_projection::{MasterProjectionInputV1, build_receipt_v1};

fn read_input() -> Result<String, String> {
    if let Some(path) = env::args().nth(1) {
        fs::read_to_string(&path).map_err(|error| format!("failed to read {path}: {error}"))
    } else {
        let mut input = String::new();
        io::stdin()
            .read_to_string(&mut input)
            .map_err(|error| format!("failed to read stdin: {error}"))?;
        Ok(input)
    }
}

fn run() -> Result<(), String> {
    let raw = read_input()?;
    let input: MasterProjectionInputV1 =
        serde_json::from_str(&raw).map_err(|error| format!("invalid input JSON: {error}"))?;
    let receipt =
        build_receipt_v1(&input).map_err(|error| format!("master projection rejected: {error}"))?;
    let output =
        serde_json::to_string_pretty(&receipt).map_err(|error| format!("serialize receipt: {error}"))?;
    println!("{output}");
    Ok(())
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("master-projection-producer: {error}");
            ExitCode::FAILURE
        }
    }
}
