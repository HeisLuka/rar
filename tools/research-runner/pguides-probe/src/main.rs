use anyhow::{Context, Result};
use pub_core::StreamPath;
use pub_escher::inspect_sp_containers;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{env, fs, path::Path};

const INTERESTING_PROPERTY_IDS: &[u16] = &[
    0x0145, // pVertices
    0x0146, // pSegmentInfo
    0x0147, // adjustValue1
    0x0148, // adjustValue2
    0x0149, // adjustValue3
    0x014A, // adjustValue4
    0x014B, // adjustValue5
    0x014C, // adjustValue6
    0x014D, // adjustValue7
    0x014E, // adjustValue8
    0x0151, // pConnectionSites
    0x0152, // pConnectionSitesDir
    0x0155, // pAdjustHandles
    0x0156, // pGuides
    0x0157, // pInscribe
];

fn sha256_hex(bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(bytes);
    h.finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>()
}

fn main() -> Result<()> {
    let mut args = env::args().skip(1);
    let pub_path = args.next().context("usage: pguides-write-probe <file.pub>")?;
    if args.next().is_some() {
        anyhow::bail!("unexpected extra arguments");
    }

    let pub_bytes = fs::read(&pub_path).with_context(|| format!("read {}", pub_path))?;
    let escher = pub_cfb::read_stream_path(&pub_path, "/Escher/EscherStm")
        .context("read /Escher/EscherStm")?;
    let stream = StreamPath("/Escher/EscherStm".into());
    let inventory = inspect_sp_containers(stream, &escher).context("inspect SP containers")?;

    let mut shapes = Vec::<Value>::new();
    let mut dynamic_candidate_count = 0usize;

    for shape in inventory.shapes {
        let Some(fsp) = shape.fsp.as_ref() else {
            continue;
        };

        let mut properties = Vec::<Value>::new();
        let mut has_guides = false;
        let mut has_adjust_handles = false;

        for fopt in &shape.fopts {
            for property in &fopt.properties {
                let property_id = property.property_id();
                if !INTERESTING_PROPERTY_IDS.contains(&property_id) {
                    continue;
                }
                has_guides |= property_id == 0x0156;
                has_adjust_handles |= property_id == 0x0155;

                let complex = property.complex_data.as_deref().map(|bytes| {
                    json!({
                        "len": bytes.len(),
                        "sha256": sha256_hex(bytes),
                        "source": property.complex_source.as_ref().map(|span| json!({
                            "offset": span.offset,
                            "len": span.len
                        }))
                    })
                });

                properties.push(json!({
                    "rec_type": fopt.rec_type,
                    "fopt_source": {
                        "offset": fopt.source.offset,
                        "len": fopt.source.len
                    },
                    "property_id": property_id,
                    "opid": property.opid,
                    "f_complex": property.f_complex(),
                    "f_bid": property.f_bid(),
                    "op_u32": property.op,
                    "op_i32": property.op as i32,
                    "entry_source": {
                        "offset": property.source.offset,
                        "len": property.source.len
                    },
                    "complex": complex
                }));
            }
        }

        if has_guides && has_adjust_handles {
            dynamic_candidate_count += 1;
        }

        if !properties.is_empty() {
            shapes.push(json!({
                "spid": fsp.spid,
                "shape_type": fsp.shape_type,
                "fsp_source": {
                    "offset": fsp.source.offset,
                    "len": fsp.source.len
                },
                "has_pguides": has_guides,
                "has_padjusthandles": has_adjust_handles,
                "properties": properties
            }));
        }
    }

    let result = json!({
        "schema": "pub-pguides-write-probe/v1",
        "file_name": Path::new(&pub_path).file_name().map(|s| s.to_string_lossy().into_owned()),
        "pub_size": pub_bytes.len(),
        "pub_sha256": sha256_hex(&pub_bytes),
        "escher_size": escher.len(),
        "escher_sha256": sha256_hex(&escher),
        "dynamic_candidate_count": dynamic_candidate_count,
        "shapes": shapes
    });

    println!("{}", serde_json::to_string_pretty(&result)?);
    Ok(())
}
