use anyhow::{Context, Result, bail};
use pub_layout::BoundedLayoutEnvironment;
use pub_model::Sha256Digest;
use pub_reader::build_mature_0x2c_source_graph;
use pub_viewer::open_mature_0x2c_geometry;
use sha2::{Digest, Sha256};
use std::{env, fs, io::Cursor};

fn main() -> Result<()> {
    let path = env::args()
        .nth(1)
        .context("usage: paint_bridge_parity <file.pub>")?;
    let bytes = fs::read(&path).with_context(|| format!("read {path}"))?;

    let digest = Sha256::digest(&bytes);
    let mut hash_bytes = [0_u8; 32];
    hash_bytes.copy_from_slice(&digest);
    let source_hash = Sha256Digest::from_bytes(hash_bytes);

    let source = build_mature_0x2c_source_graph(Cursor::new(bytes.as_slice()), source_hash)
        .context("build source graph")?;
    let viewer = open_mature_0x2c_geometry(
        &bytes,
        BoundedLayoutEnvironment {
            engine_revision: "paint-bridge-parity-v1".to_owned(),
            font_set_fingerprint: "paint-bridge-parity-v1".to_owned(),
            resource_fingerprint: "paint-bridge-parity-v1".to_owned(),
        },
    )
    .context("open Viewer geometry")?;

    let mut legacy_paintable = 0_usize;
    let mut compared = 0_usize;
    let mut mismatches = Vec::new();

    for node in source.graph.nodes.values() {
        let paint = &node.payload.explicit_paint;
        let legacy_fill = (paint.fill.solid && paint.fill.visible == Some(true))
            .then_some(paint.fill.color_rgb)
            .flatten();
        let legacy_line = match (
            paint.line.visible,
            paint.line.color_rgb,
            paint.line.width_emu,
        ) {
            (Some(true), Some(rgb), Some(width_emu)) if width_emu > 0 => Some((rgb, width_emu)),
            _ => None,
        };
        if legacy_fill.is_none() && legacy_line.is_none() {
            continue;
        }
        legacy_paintable += 1;

        let projected = viewer
            .paints
            .iter()
            .find(|candidate| candidate.node_id == node.header.id);
        let Some(projected) = projected else {
            mismatches.push(format!(
                "{}: legacy paint exists but canonical Viewer paint is absent",
                node.header.id.as_canonical()
            ));
            continue;
        };
        compared += 1;

        let projected_line = projected
            .solid_line
            .as_ref()
            .map(|line| (line.rgb, line.width_emu));
        if projected.solid_fill_rgb != legacy_fill || projected_line != legacy_line {
            mismatches.push(format!(
                "{}: legacy fill={legacy_fill:?} line={legacy_line:?}; canonical fill={:?} line={projected_line:?}",
                node.header.id.as_canonical(),
                projected.solid_fill_rgb
            ));
        }
    }

    let unexpected = viewer
        .paints
        .iter()
        .filter(|candidate| {
            !source.graph.nodes.values().any(|node| {
                if node.header.id != candidate.node_id {
                    return false;
                }
                let paint = &node.payload.explicit_paint;
                let fill = paint.fill.solid
                    && paint.fill.visible == Some(true)
                    && paint.fill.color_rgb.is_some();
                let line = matches!(
                    (
                        paint.line.visible,
                        paint.line.color_rgb,
                        paint.line.width_emu,
                    ),
                    (Some(true), Some(_), Some(width_emu)) if width_emu > 0
                );
                fill || line
            })
        })
        .count();

    println!(
        "PAINT_BRIDGE_PARITY\tlegacy_paintable={legacy_paintable}\tcanonical_paints={}\tcompared={compared}\tmismatches={}\tunexpected={unexpected}",
        viewer.paints.len(),
        mismatches.len()
    );
    for mismatch in &mismatches {
        eprintln!("PAINT_BRIDGE_MISMATCH\t{mismatch}");
    }

    if !mismatches.is_empty() || unexpected != 0 || viewer.paints.len() != legacy_paintable {
        bail!("canonical paint bridge parity failed");
    }
    Ok(())
}
