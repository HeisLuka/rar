use chaptera_update_engine::UpdateEngine;
use chaptera_update_handoff::{
    read_control_request, receipt_path, started_path, validate_request_against_engine,
    write_control_receipt, ControlReceipt, CONTROL_MODE_ARG, CONTROL_RECEIPT_SCHEMA_VERSION,
};
use chaptera_update_orchestrator::InstallLock;
use std::fs;
use std::path::PathBuf;

fn main() {
    if let Err(err) = run() {
        eprintln!("chaptera update control probe failed: {err}");
        std::process::exit(2);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args_os();
    let _exe = args.next();
    let mode = args.next().ok_or("missing control mode")?;
    if mode != CONTROL_MODE_ARG {
        return Err(format!("unexpected control mode: {:?}", mode).into());
    }
    let request_path = PathBuf::from(args.next().ok_or("missing request path")?);
    if args.next().is_some() {
        return Err("unexpected extra control arguments".into());
    }

    let request = read_control_request(&request_path)?;

    // This marker is written before blocking on the install lock so acceptance
    // can prove the child started but could not yet assume update ownership.
    fs::write(started_path(&request_path), b"started\n")?;

    let _lock = InstallLock::acquire(&request.install_root)?;
    let engine = UpdateEngine::new(&request.install_root);
    validate_request_against_engine(&request, &engine)?;

    let receipt = ControlReceipt {
        schema_version: CONTROL_RECEIPT_SCHEMA_VERSION.to_owned(),
        transaction_id: request.transaction_id,
        pid: std::process::id(),
        executable: std::env::current_exe()?,
    };
    write_control_receipt(&receipt_path(&request_path), &receipt)?;
    Ok(())
}
