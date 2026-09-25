use std::{env, fs, process::ExitCode};

use pub_editor::{
    EditorEditableTarget, editor_copy_ledger_snapshot_v1, open_mature_0x2c_editor,
    reset_editor_copy_ledger_v1,
};
use pub_model::Sha256Digest;

fn run() -> Result<(), String> {
    let mut args = env::args().skip(1);
    let path = args
        .next()
        .ok_or_else(|| "usage: copy_ledger_replace_image_export FILE.pub SHA256".to_owned())?;
    let source_hash: Sha256Digest = args
        .next()
        .ok_or_else(|| "usage: copy_ledger_replace_image_export FILE.pub SHA256".to_owned())?
        .parse()
        .map_err(|error| format!("invalid SHA256: {error}"))?;
    if args.next().is_some() {
        return Err("usage: copy_ledger_replace_image_export FILE.pub SHA256".to_owned());
    }

    let bytes = fs::read(&path).map_err(|error| format!("read {path}: {error}"))?;
    let mut session =
        open_mature_0x2c_editor(&bytes, source_hash).map_err(|error| error.to_string())?;

    // The editor validates the PNG signature at import. A tiny deterministic
    // payload keeps the hosted probe source-neutral while exercising the exact
    // replacement/export path used by real runs.
    let replacement = b"\x89PNG\r\n\x1a\ncopy-ledger-replacement-v1".to_vec();
    let asset = session
        .import_replacement_asset("image/png", replacement)
        .map_err(|error| error.to_string())?;

    let node_id = session
        .graph()
        .nodes
        .iter()
        .find_map(|(node_id, node)| {
            (node.payload.image_slot.is_some()
                && node.payload.explicit_image_crop.is_none()
                && session.can_replace_image(*node_id, asset).is_ok())
            .then_some(*node_id)
        })
        .ok_or_else(|| "public fixture has no ReplaceImage-admissible node".to_owned())?;

    session
        .replace_image(node_id, asset)
        .map_err(|error| error.to_string())?;

    reset_editor_copy_ledger_v1();
    let export = session
        .export_editable(EditorEditableTarget::Idml, "copy-ledger-public-fixture")
        .map_err(|error| error.to_string())?;
    let snapshot = editor_copy_ledger_snapshot_v1();

    println!(
        "{{"scenario":"replace_image_export","export_bytes":{},"replacement_image_clone_bytes":{},"replacement_image_clone_instances":{},"editable_serialization_bytes":{},"editable_serialization_instances":{}}}",
        export.bytes.len(),
        snapshot.replacement_image_clone_bytes,
        snapshot.replacement_image_clone_instances,
        snapshot.editable_serialization_bytes,
        snapshot.editable_serialization_instances,
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
