use std::{
    env, fs,
    io::{self, Read},
    process::ExitCode,
};

use chaptera_layout_projection::{CmoStorySlotFlowInputV1, resolve_cmo_slot_flow_v1};
use pub_model::PubProjectionContextV1;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PacketInputV1 {
    projection_context: PubProjectionContextV1,
    slot_flow: CmoStorySlotFlowInputV1,
}

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
    let packet: PacketInputV1 =
        serde_json::from_str(&raw).map_err(|error| format!("invalid input JSON: {error}"))?;
    let output = resolve_cmo_slot_flow_v1(&packet.projection_context, &packet.slot_flow)
        .map_err(|error| format!("Cmo slot-flow rejected: {error}"))?;
    let output = serde_json::to_string_pretty(&output)
        .map_err(|error| format!("serialize Cmo slot-flow output: {error}"))?;
    println!("{output}");
    Ok(())
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("cmo-slot-flow-packet: {error}");
            ExitCode::FAILURE
        }
    }
}
