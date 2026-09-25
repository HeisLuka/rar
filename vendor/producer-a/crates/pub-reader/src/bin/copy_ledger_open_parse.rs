use std::{env, fs::File, process::ExitCode};

use pub_model::Sha256Digest;
use pub_reader::{
    build_mature_0x2c_source_graph, pub_reader_copy_ledger_snapshot_v1,
    reset_pub_reader_copy_ledger_v1,
};

fn run() -> Result<(), String> {
    let mut args = env::args().skip(1);
    let path = args
        .next()
        .ok_or_else(|| "usage: copy_ledger_open_parse FILE.pub SHA256".to_owned())?;
    let source_hash: Sha256Digest = args
        .next()
        .ok_or_else(|| "usage: copy_ledger_open_parse FILE.pub SHA256".to_owned())?
        .parse()
        .map_err(|error| format!("invalid SHA256: {error}"))?;
    if args.next().is_some() {
        return Err("usage: copy_ledger_open_parse FILE.pub SHA256".to_owned());
    }

    reset_pub_reader_copy_ledger_v1();
    let file = File::open(&path).map_err(|error| format!("open {path}: {error}"))?;
    let build = build_mature_0x2c_source_graph(file, source_hash)
        .map_err(|error| format!("parse {path}: {error:#}"))?;
    let snapshot = pub_reader_copy_ledger_snapshot_v1();

    println!(
        "{{"scenario":"open_parse","nodes":{},"stories":{},"file_buffer_bytes":{},"file_buffer_instances":{},"contents_stream_bytes":{},"contents_stream_instances":{},"quill_stream_bytes":{},"quill_stream_instances":{},"escher_stream_bytes":{},"escher_stream_instances":{}}}",
        build.graph.nodes.len(),
        build.graph.stories.len(),
        snapshot.file_buffer_bytes,
        snapshot.file_buffer_instances,
        snapshot.contents_stream_bytes,
        snapshot.contents_stream_instances,
        snapshot.quill_stream_bytes,
        snapshot.quill_stream_instances,
        snapshot.escher_stream_bytes,
        snapshot.escher_stream_instances,
    );
    Ok(())
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}
