use anyhow::{Context, Result};
use pub_model::{Sha256Digest, to_cdm_debug_json_v0_1};
use std::{env, fs, io::{self, Cursor, Write}};

fn main() -> Result<()> {
    let path = env::args().nth(1).context("fixture path argument missing")?;
    let source_hash = env::args().nth(2).context("source hash argument missing")?;
    let digest: Sha256Digest = source_hash.parse().context("parse source SHA-256")?;
    let bytes = fs::read(path).context("read pinned PUB fixture")?;
    let source = pub_reader::build_mature_0x2c_source_graph(Cursor::new(bytes), digest)
        .context("build mature-0x2c source graph")?;
    let resolved = pub_reader::resolve_pub_source_graph(&source.graph)
        .context("resolve mature-0x2c source graph")?;
    let canonical = to_cdm_debug_json_v0_1(&resolved.graph)
        .context("serialize canonical resolved graph")?;
    io::stdout().write_all(&canonical)?;
    Ok(())
}
