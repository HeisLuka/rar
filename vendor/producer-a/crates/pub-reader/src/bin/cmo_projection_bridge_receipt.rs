use anyhow::{Context, Result, bail};
use pub_model::Sha256Digest;
use pub_reader::{
    build_mature_0x2c_cmo_projection_bridge_v1, build_mature_0x2c_source_graph,
    resolve_pub_source_graph,
};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{fs, io::Cursor, path::PathBuf};

fn main() -> Result<()> {
    let mut args = std::env::args_os().skip(1);
    let input = args.next().map(PathBuf::from).context("missing input PUB path")?;
    let output = args.next().map(PathBuf::from).context("missing output receipt path")?;
    if args.next().is_some() {
        bail!("usage: cmo_projection_bridge_receipt <input.pub> <output.json>");
    }

    let bytes = fs::read(&input).with_context(|| format!("read {}", input.display()))?;
    let digest = Sha256::digest(&bytes);
    let mut sha = [0_u8; 32];
    sha.copy_from_slice(&digest);
    let source_hash = Sha256Digest::from_bytes(sha);

    let source = build_mature_0x2c_source_graph(Cursor::new(bytes.as_slice()), source_hash)
        .context("build active Reader source graph")?;
    let resolved =
        resolve_pub_source_graph(&source.graph).context("resolve active Reader graph")?;
    let bridge = build_mature_0x2c_cmo_projection_bridge_v1(
        &bytes,
        source_hash,
        &source.graph,
        &resolved.graph,
    )
    .context("build active Reader Cmo authority bridge")?;

    let receipt = json!({
        "schema": "chaptera.reader-cmo-authority-bridge.v1",
        "source_sha256": source_hash.to_string(),
        "active_graph_identity_parity": bridge.active_graph_identity_parity,
        "relation_count": bridge.output.context.cmo_relations.len(),
        "target_count": bridge.output.receipt.targets.len(),
        "relations": bridge.output.context.cmo_relations,
        "targets": bridge.output.receipt.targets,
        "invariants": bridge.output.receipt.invariants,
    });

    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("create {}", parent.display()))?;
    }
    fs::write(
        &output,
        serde_json::to_vec_pretty(&receipt).context("serialize Cmo bridge receipt")?,
    )
    .with_context(|| format!("write {}", output.display()))?;

    println!(
        "READER CMO AUTHORITY BRIDGE PASS relations={} targets={} identity_parity={}",
        receipt["relation_count"],
        receipt["target_count"],
        receipt["active_graph_identity_parity"],
    );
    Ok(())
}
