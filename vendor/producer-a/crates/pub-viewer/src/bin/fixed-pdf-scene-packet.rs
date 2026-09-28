use anyhow::{Context, Result};
use pub_viewer::{open_mature_0x2c_geometry, viewer_geometry_environment_v0_1};
use serde_json::json;
use std::{env, fs, path::PathBuf};

fn hex_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write as _;
        let _ = write!(out, "{byte:02x}");
    }
    out
}

fn main() -> Result<()> {
    let mut args = env::args_os().skip(1);
    let source = PathBuf::from(
        args.next()
            .context("usage: fixed-pdf-scene-packet SOURCE.pub OUTPUT.json")?,
    );
    let output = PathBuf::from(
        args.next()
            .context("usage: fixed-pdf-scene-packet SOURCE.pub OUTPUT.json")?,
    );
    if args.next().is_some() {
        anyhow::bail!("usage: fixed-pdf-scene-packet SOURCE.pub OUTPUT.json");
    }

    let bytes = fs::read(&source).with_context(|| format!("read {}", source.display()))?;
    let visual = open_mature_0x2c_geometry(&bytes, viewer_geometry_environment_v0_1())
        .context("open PUB through current Viewer geometry authority")?;

    let images = visual
        .images
        .iter()
        .map(|image| {
            json!({
                "resource_id": image.resource_id,
                "mime": image.mime,
                "node_ids": image.node_ids,
                "bytes_hex": hex_encode(&image.bytes),
            })
        })
        .collect::<Vec<_>>();

    let packet = json!({
        "schema_version": "chaptera.fixed-pdf-scene-packet.v1",
        "source_sha256": visual.document.source.source_hash,
        "scene": &visual.scene,
        "paints": &visual.paints,
        "story_frames": &visual.story_frames,
        "stories": &visual.document.stories,
        "images": images,
        "viewer_diagnostic_codes": visual
            .document
            .diagnostics
            .iter()
            .map(|diagnostic| diagnostic.code.clone())
            .collect::<Vec<_>>(),
    });

    if let Some(parent) = output.parent().filter(|parent| !parent.as_os_str().is_empty()) {
        fs::create_dir_all(parent)
            .with_context(|| format!("create packet directory {}", parent.display()))?;
    }
    fs::write(
        &output,
        serde_json::to_vec_pretty(&packet).context("serialize fixed-PDF Scene packet")?,
    )
    .with_context(|| format!("write {}", output.display()))?;

    println!(
        "FIXED PDF SCENE PACKET PASS pages={} surfaces={} nodes={} stories={} frames={} images={}",
        visual.document.pages.len(),
        visual.scene.surfaces.len(),
        visual.scene.nodes.len(),
        visual.document.stories.len(),
        visual.story_frames.len(),
        visual.images.len(),
    );

    Ok(())
}
