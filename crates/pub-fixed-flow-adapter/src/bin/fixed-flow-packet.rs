use std::{
    env, fs,
    io::{self, Read},
    process::ExitCode,
};

use pub_fixed_flow_adapter::{ShapedFlowInputV1, build_receipt_v1, materialize_fixed_runs_v1};
use serde_json::json;

const PACKET_VERSION_V1: &str = "chaptera.fixed-flow-adapter-packet.v1";

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
    let input: ShapedFlowInputV1 =
        serde_json::from_str(&raw).map_err(|error| format!("invalid input JSON: {error}"))?;
    let runs = materialize_fixed_runs_v1(&input)
        .map_err(|error| format!("fixed-flow adapter rejected: {error}"))?;
    let receipt = build_receipt_v1(&input)
        .map_err(|error| format!("fixed-flow adapter rejected: {error}"))?;

    let run_values = runs
        .iter()
        .map(|run| {
            json!({
                "run_index": run.run_index,
                "frame_node_id": run.frame_node_id,
                "story_id": run.story_id,
                "scalar_base": run.scalar_base,
                "scalar_end": run.scalar_end,
                "units_per_em": run.units_per_em,
                "measured_width": run.measured_width,
                "baseline_x": run.baseline_x,
                "baseline_y": run.baseline_y,
                "glyphs": run.glyphs,
            })
        })
        .collect::<Vec<_>>();

    let packet = json!({
        "protocol_version": PACKET_VERSION_V1,
        "runs": run_values,
        "receipt": receipt,
    });
    let output = serde_json::to_string_pretty(&packet)
        .map_err(|error| format!("serialize adapter packet: {error}"))?;
    println!("{output}");
    Ok(())
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("fixed-flow-packet: {error}");
            ExitCode::FAILURE
        }
    }
}
