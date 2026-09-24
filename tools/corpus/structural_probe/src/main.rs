use anyhow::{Context, Result};
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::fs;
use std::path::Path;

const SCHEMA: &str = "rar-pub-structural-probe/v1";
const YAB_PIN: &str = "2416bf1af401caa997e588b41285816422a7d59b";

#[derive(Debug, Serialize)]
struct StreamDigest { path: String, len: u64, sha256: String }

#[derive(Debug, Serialize)]
struct MatureSummary {
    candidate_count: usize,
    page_count_with_candidates: usize,
    diagnostic_count: usize,
    shape_type_counts: BTreeMap<String, usize>,
    diagnostic_code_counts: BTreeMap<String, usize>,
    layout_fingerprint_sha256: String,
}

#[derive(Debug, Serialize)]
struct Probe {
    schema: &'static str,
    analyzer_pin: &'static str,
    source_sha256: String,
    byte_len: u64,
    stream_count: usize,
    streams: Vec<StreamDigest>,
    topology_fingerprint_sha256: String,
    content_topology_fingerprint_sha256: String,
    family: String,
    structural_base_status: String,
    structural_base_error: Option<String>,
    mature: Option<MatureSummary>,
}

fn hex_sha(bytes: &[u8]) -> String { format!("{:x}", Sha256::digest(bytes)) }

fn hash_lines(lines: impl IntoIterator<Item = String>) -> String {
    let mut hasher = Sha256::new();
    for line in lines {
        hasher.update(line.as_bytes());
        hasher.update(b"\n");
    }
    format!("{:x}", hasher.finalize())
}

fn main() -> Result<()> {
    let path = env::args().nth(1).context("usage: rar-pub-structural-probe FILE.pub")?;
    let path = Path::new(&path);
    let bytes = fs::read(path).with_context(|| format!("read {}", path.display()))?;
    let source_sha256 = hex_sha(&bytes);

    let inventory = pub_cfb::inspect_path(path).context("inspect CFB")?;
    let mut streams = Vec::new();
    for entry in inventory.entries.iter().filter(|e| matches!(e.kind, pub_cfb::EntryKind::Stream)) {
        let payload = pub_cfb::read_stream_path(path, &entry.path)
            .with_context(|| format!("read stream {}", entry.path))?;
        streams.push(StreamDigest { path: entry.path.clone(), len: entry.len, sha256: hex_sha(&payload) });
    }
    streams.sort_by(|a, b| a.path.cmp(&b.path));

    let topology_fingerprint_sha256 =
        hash_lines(streams.iter().map(|s| format!("{}\t{}", s.path, s.len)));
    let content_topology_fingerprint_sha256 =
        hash_lines(streams.iter().map(|s| format!("{}\t{}\t{}", s.path, s.len, s.sha256)));

    let (family, structural_base_status, structural_base_error, mature) =
        match pub_reader::build_mature_0x2c_structural_base_manifest(&bytes) {
            Ok(manifest) => {
                let mut shape_type_counts = BTreeMap::new();
                let mut pages = BTreeSet::new();
                let mut layout_rows = Vec::new();
                for c in &manifest.candidates {
                    *shape_type_counts.entry(format!("0x{:04x}", c.officeart_shape_type)).or_default() += 1;
                    pages.insert(format!("{:?}", c.page_id));
                    layout_rows.push(format!(
                        "{}\t{}\t{}\t{}\t{}",
                        c.officeart_shape_type,
                        c.bounds_emu.x.0, c.bounds_emu.y.0,
                        c.bounds_emu.width.0, c.bounds_emu.height.0
                    ));
                }
                layout_rows.sort();

                let mut diagnostic_code_counts = BTreeMap::new();
                for d in &manifest.diagnostics {
                    let value = serde_json::to_value(d).context("serialize diagnostic")?;
                    let code = value.get("code").and_then(Value::as_str).unwrap_or("unknown").to_owned();
                    *diagnostic_code_counts.entry(code).or_default() += 1;
                }

                let mut fingerprint_rows = vec![format!("topology={topology_fingerprint_sha256}")];
                fingerprint_rows.extend(layout_rows);
                for (code, count) in &diagnostic_code_counts {
                    fingerprint_rows.push(format!("diag={code}:{count}"));
                }

                (
                    "mature_0x2c".to_owned(), "ok".to_owned(), None,
                    Some(MatureSummary {
                        candidate_count: manifest.candidates.len(),
                        page_count_with_candidates: pages.len(),
                        diagnostic_count: manifest.diagnostics.len(),
                        shape_type_counts,
                        diagnostic_code_counts,
                        layout_fingerprint_sha256: hash_lines(fingerprint_rows),
                    })
                )
            }
            Err(err) => (
                "cfb_unsupported_or_legacy".to_owned(),
                "unsupported".to_owned(),
                Some(format!("{err:#}")),
                None
            ),
        };

    let probe = Probe {
        schema: SCHEMA,
        analyzer_pin: YAB_PIN,
        source_sha256,
        byte_len: u64::try_from(bytes.len()).context("byte length")?,
        stream_count: streams.len(),
        streams,
        topology_fingerprint_sha256,
        content_topology_fingerprint_sha256,
        family,
        structural_base_status,
        structural_base_error,
        mature,
    };

    println!("{}", serde_json::to_string_pretty(&probe)?);
    Ok(())
}
