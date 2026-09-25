use pub_core::StreamPath;
use pub_model::Sha256Digest;
use pub_quill::parse_confirmed_story_catalog;
use pub_reader::{
    QUILL_STREAM_PATH, build_mature_0x2c_source_graph, derive_pub_story_id,
    resolve_pub_source_graph,
};
use pub_writer::{
    PUB_WRITER_PROBE_VERSION_V0_1, StoryTextWriteProbeRequest,
    materialize_mature_0x2c_story_text_pub_candidate,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::error::Error;
use std::fs;
use std::io::Cursor;
use std::path::{Path, PathBuf};

const STORY_SYID: u32 = 4;
const CONTROLLED_MARKER: &str = "345678";
const MANIFEST_SCHEMA: &str = "chaptera.pub-native-story-handoff.v1";

fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(2);
    }
}

fn run() -> Result<(), Box<dyn Error>> {
    let mut args = std::env::args().skip(1);
    match args.next().as_deref() {
        Some("generate") => {
            let output_dir = parse_flag_path(&mut args, "--output-dir")?;
            if args.next().is_some() {
                return Err("unexpected arguments after --output-dir".into());
            }
            generate(&output_dir)
        }
        Some("verify") => {
            let pub_path = parse_flag_path(&mut args, "--pub")?;
            let manifest_path = parse_flag_path(&mut args, "--manifest")?;
            if args.next().is_some() {
                return Err("unexpected arguments after --manifest".into());
            }
            verify(&pub_path, &manifest_path)
        }
        _ => Err(
            "usage: pub-story-native-handoff generate --output-dir DIR | verify --pub FILE.pub --manifest handoff.json"
                .into(),
        ),
    }
}

fn parse_flag_path(
    args: &mut impl Iterator<Item = String>,
    expected: &str,
) -> Result<PathBuf, Box<dyn Error>> {
    let flag = args.next().ok_or_else(|| format!("missing {expected}"))?;
    if flag != expected {
        return Err(format!("expected {expected}, got {flag}").into());
    }
    let value = args
        .next()
        .ok_or_else(|| format!("missing value for {expected}"))?;
    Ok(PathBuf::from(value))
}

fn generate(output_dir: &Path) -> Result<(), Box<dyn Error>> {
    fs::create_dir_all(output_dir)?;

    let source = sample3_pub()?;
    let source_hash = sha256_digest(&source);
    let source_hash_text = source_hash.to_string();

    let quill = pub_cfb::read_stream_reader(Cursor::new(&source), QUILL_STREAM_PATH)?;
    let catalog = parse_confirmed_story_catalog(StreamPath(QUILL_STREAM_PATH.into()), &quill)?;
    let story = catalog
        .stories
        .iter()
        .find(|story| story.syid.0 == STORY_SYID)
        .ok_or("controlled SYID 4 Story is missing")?;
    let before = String::from_utf16(&utf16le_units(&story.utf16le)?)?;
    if before.match_indices(CONTROLLED_MARKER).count() != 1 {
        return Err("controlled Story must contain marker 345678 exactly once".into());
    }
    let after = before.replacen(CONTROLLED_MARKER, "", 1);

    let request = StoryTextWriteProbeRequest {
        source_hash,
        story_id: derive_pub_story_id(&source_hash, STORY_SYID)?,
        before: before.clone(),
        after: after.clone(),
    };
    let candidate = materialize_mature_0x2c_story_text_pub_candidate(&source, &request)?;

    let source_path = output_dir.join("source.pub");
    let candidate_path = output_dir.join("candidate.pub");
    let manifest_path = output_dir.join("handoff.json");
    fs::write(&source_path, &source)?;
    fs::write(&candidate_path, &candidate.bytes)?;

    let manifest = json!({
        "schema": MANIFEST_SCHEMA,
        "writer_probe_version": PUB_WRITER_PROBE_VERSION_V0_1,
        "fixture": "pub-quill/tests/fixtures/Sample3.pub.b64",
        "source_sha256": source_hash_text,
        "candidate_sha256": candidate.output_hash.to_string(),
        "story_syid": STORY_SYID,
        "before_text_sha256": sha256_text(&before),
        "after_text_sha256": sha256_text(&after),
        "before_utf16_len": before.encode_utf16().count(),
        "after_utf16_len": after.encode_utf16().count(),
        "mutation": {
            "kind": "bounded_story_text_delete",
            "removed_utf16_units": CONTROLLED_MARKER.encode_utf16().count()
        },
        "native_target": {
            "publisher_family": "Publisher 2019",
            "application_version": "16.0",
            "build_prefix": "12527"
        }
    });
    fs::write(&manifest_path, serde_json::to_vec_pretty(&manifest)?)?;

    let verified = verify_pub_bytes(&candidate.bytes, &manifest)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "status": "generated",
            "source": source_path,
            "candidate": candidate_path,
            "manifest": manifest_path,
            "offline_verify": verified
        }))?
    );
    Ok(())
}

fn verify(pub_path: &Path, manifest_path: &Path) -> Result<(), Box<dyn Error>> {
    let manifest: Value = serde_json::from_slice(&fs::read(manifest_path)?)?;
    if manifest.get("schema").and_then(Value::as_str) != Some(MANIFEST_SCHEMA) {
        return Err("unsupported handoff manifest schema".into());
    }
    let bytes = fs::read(pub_path)?;
    let verified = verify_pub_bytes(&bytes, &manifest)?;
    println!("{}", serde_json::to_string_pretty(&verified)?);
    Ok(())
}

fn verify_pub_bytes(bytes: &[u8], manifest: &Value) -> Result<Value, Box<dyn Error>> {
    let expected_syid = manifest
        .get("story_syid")
        .and_then(Value::as_u64)
        .ok_or("manifest story_syid is missing")? as u32;
    let expected_text_hash = manifest
        .get("after_text_sha256")
        .and_then(Value::as_str)
        .ok_or("manifest after_text_sha256 is missing")?;
    let expected_utf16_len = manifest
        .get("after_utf16_len")
        .and_then(Value::as_u64)
        .ok_or("manifest after_utf16_len is missing")? as usize;

    let output_hash = sha256_digest(bytes);
    let reopened = build_mature_0x2c_source_graph(Cursor::new(bytes), output_hash)?;
    let resolved = resolve_pub_source_graph(&reopened.graph)?;
    let story_id = derive_pub_story_id(&output_hash, expected_syid)?;
    let story = resolved
        .graph
        .stories
        .get(&story_id)
        .ok_or("verified PUB does not contain expected Story")?;

    let actual_hash = sha256_text(&story.text);
    let actual_utf16_len = story.text.encode_utf16().count();
    if actual_hash != expected_text_hash {
        return Err(format!(
            "Story text hash mismatch after PUB lifecycle: expected {expected_text_hash}, got {actual_hash}"
        )
        .into());
    }
    if actual_utf16_len != expected_utf16_len {
        return Err(format!(
            "Story UTF-16 length mismatch after PUB lifecycle: expected {expected_utf16_len}, got {actual_utf16_len}"
        )
        .into());
    }

    Ok(json!({
        "status": "valid",
        "pub_sha256": output_hash.to_string(),
        "story_syid": expected_syid,
        "story_text_sha256": actual_hash,
        "story_utf16_len": actual_utf16_len
    }))
}

fn sample3_pub() -> Result<Vec<u8>, Box<dyn Error>> {
    decode_base64(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../pub-quill/tests/fixtures/Sample3.pub.b64"
    )))
}

fn decode_base64(text: &str) -> Result<Vec<u8>, Box<dyn Error>> {
    let cleaned = text
        .bytes()
        .filter(|byte| !byte.is_ascii_whitespace())
        .collect::<Vec<_>>();
    if cleaned.len() % 4 != 0 {
        return Err("base64 fixture length is not divisible by four".into());
    }

    let mut output = Vec::with_capacity(cleaned.len() / 4 * 3);
    for quartet in cleaned.chunks_exact(4) {
        let a = base64_value(quartet[0])?;
        let b = base64_value(quartet[1])?;
        let c = if quartet[2] == b'=' {
            0
        } else {
            base64_value(quartet[2])?
        };
        let d = if quartet[3] == b'=' {
            0
        } else {
            base64_value(quartet[3])?
        };
        output.push((a << 2) | (b >> 4));
        if quartet[2] != b'=' {
            output.push((b << 4) | (c >> 2));
        }
        if quartet[3] != b'=' {
            output.push((c << 6) | d);
        }
    }
    Ok(output)
}

fn base64_value(byte: u8) -> Result<u8, Box<dyn Error>> {
    Ok(match byte {
        b'A'..=b'Z' => byte - b'A',
        b'a'..=b'z' => byte - b'a' + 26,
        b'0'..=b'9' => byte - b'0' + 52,
        b'+' => 62,
        b'/' => 63,
        other => return Err(format!("invalid base64 byte {other:#x}").into()),
    })
}

fn utf16le_units(bytes: &[u8]) -> Result<Vec<u16>, Box<dyn Error>> {
    if bytes.len() % 2 != 0 {
        return Err("Story UTF-16LE byte length is odd".into());
    }
    Ok(bytes
        .chunks_exact(2)
        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
        .collect())
}

fn sha256_digest(bytes: &[u8]) -> Sha256Digest {
    let digest = Sha256::digest(bytes);
    let mut value = [0_u8; 32];
    value.copy_from_slice(&digest);
    Sha256Digest::from_bytes(value)
}

fn sha256_text(text: &str) -> String {
    let digest = Sha256::digest(text.as_bytes());
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}
