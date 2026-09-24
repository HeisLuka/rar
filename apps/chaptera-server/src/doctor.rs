use crate::{runtime_error::RuntimeError, state::AppState};

pub fn render(state: &AppState) -> Result<String, RuntimeError> {
    serde_json::to_string_pretty(&state.readiness_report()).map_err(|error| {
        RuntimeError::new(
            "doctor_serialization_failed",
            format!("could not serialize readiness report: {error}"),
        )
    })
}

pub fn run(state: &AppState) -> Result<(), RuntimeError> {
    let report = state.readiness_report();
    let rendered = serde_json::to_string_pretty(&report).map_err(|error| {
        RuntimeError::new(
            "doctor_serialization_failed",
            format!("could not serialize readiness report: {error}"),
        )
    })?;

    println!("{rendered}");

    if report.ready {
        Ok(())
    } else {
        Err(RuntimeError::new(
            "runtime_not_ready",
            "one or more required runtime producers are not connected",
        ))
    }
}
