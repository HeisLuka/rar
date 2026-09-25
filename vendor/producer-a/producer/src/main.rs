use anyhow::{Context, Result};
use pub_model::{Sha256Digest, to_cdm_debug_json_v0_1};
use std::{env, fs, io::Cursor};

const SAMPLE_HASH: &str =
    "6a825ba26ba35d6e885acdc62e859591ed37cb0ff7480b554b9cb362b644dfcf";

fn emit_viewer(path: &str) -> Result<()> {
    let bytes = fs::read(path).context("read pinned PUB fixture")?;
    let visual = pub_viewer::open_mature_0x2c_geometry(
        &bytes,
        pub_viewer::viewer_geometry_environment_v0_1(),
    )
    .context("open mature-0x2c Viewer geometry")?;
    print!("{}", serde_json::to_string(&visual)?);
    Ok(())
}

fn emit_resolved_graph(path: &str) -> Result<()> {
    let bytes = fs::read(path).context("read pinned PUB fixture")?;
    let source_hash: Sha256Digest = SAMPLE_HASH.parse().expect("pinned SampleNewsletter SHA-256");
    let source = pub_reader::build_mature_0x2c_source_graph(
        Cursor::new(bytes.as_slice()),
        source_hash,
    )
    .context("build mature-0x2c SourceGraph")?;
    let resolved =
        pub_reader::resolve_pub_source_graph(&source.graph).context("resolve PUB SourceGraph")?;
    let canonical = to_cdm_debug_json_v0_1(&resolved.graph)
        .context("serialize canonical resolved graph")?;
    std::io::Write::write_all(&mut std::io::stdout(), &canonical)?;
    Ok(())
}

fn main() -> Result<()> {
    let mut args = env::args().skip(1);
    let first = args.next().context("fixture path or mode argument missing")?;
    if first == "resolved-graph" {
        let path = args.next().context("fixture path argument missing")?;
        if args.next().is_some() {
            anyhow::bail!("unexpected extra arguments");
        }
        return emit_resolved_graph(&path);
    }
    if args.next().is_some() {
        anyhow::bail!("unexpected extra arguments");
    }
    emit_viewer(&first)
}
