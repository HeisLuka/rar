use crate::{
    StoryTextPubCandidate, StoryTextPubMaterializationBlocked, StoryTextWriteProbeRequest,
    materialize_mature_0x2c_story_text_pub_candidate,
};
use pub_cfb::{CfbInventory, EntryKind};
use pub_model::Sha256Digest;
use pub_reader::QUILL_STREAM_PATH;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fmt;
use std::io::Cursor;

pub const PUB_SEEDED_BOOTSTRAP_REPORT_SCHEMA_V0_1: &str = "pub-seeded-bootstrap-v0.1";
pub const PUB_SEEDED_NEW_DOCUMENT_STRUCTURAL_GATE: &str = "STRUCT-WRITER-REPLAY-01";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PubSeedStreamDigest {
    pub path: String,
    pub len: u64,
    pub sha256: Sha256Digest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PubSeedManifest {
    pub schema_version: &'static str,
    pub file_sha256: Sha256Digest,
    pub inventory: CfbInventory,
    pub streams: Vec<PubSeedStreamDigest>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PubSeedManifestError {
    CfbInventory {
        detail: String,
    },
    StreamRead {
        path: String,
        detail: String,
    },
    StreamLengthMismatch {
        path: String,
        inventory_len: u64,
        actual_len: u64,
    },
}

impl fmt::Display for PubSeedManifestError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CfbInventory { detail } => {
                write!(
                    formatter,
                    "не удалось построить CFB inventory seed PUB: {detail}"
                )
            }
            Self::StreamRead { path, detail } => {
                write!(
                    formatter,
                    "не удалось прочитать seed stream {path}: {detail}"
                )
            }
            Self::StreamLengthMismatch {
                path,
                inventory_len,
                actual_len,
            } => write!(
                formatter,
                "CFB inventory length не совпадает с прочитанным stream {path}: inventory={inventory_len}, actual={actual_len}"
            ),
        }
    }
}

impl std::error::Error for PubSeedManifestError {}

pub fn inspect_pub_seed_manifest(seed_pub: &[u8]) -> Result<PubSeedManifest, PubSeedManifestError> {
    let inventory = pub_cfb::inspect_reader(Cursor::new(seed_pub)).map_err(|error| {
        PubSeedManifestError::CfbInventory {
            detail: format!("{error:#}"),
        }
    })?;

    let mut streams = Vec::new();
    for entry in inventory
        .entries
        .iter()
        .filter(|entry| entry.kind == EntryKind::Stream)
    {
        let bytes =
            pub_cfb::read_stream_reader(Cursor::new(seed_pub), &entry.path).map_err(|error| {
                PubSeedManifestError::StreamRead {
                    path: entry.path.clone(),
                    detail: format!("{error:#}"),
                }
            })?;
        let actual_len = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
        if actual_len != entry.len {
            return Err(PubSeedManifestError::StreamLengthMismatch {
                path: entry.path.clone(),
                inventory_len: entry.len,
                actual_len,
            });
        }
        streams.push(PubSeedStreamDigest {
            path: entry.path.clone(),
            len: entry.len,
            sha256: sha256_digest(&bytes),
        });
    }

    Ok(PubSeedManifest {
        schema_version: PUB_SEEDED_BOOTSTRAP_REPORT_SCHEMA_V0_1,
        file_sha256: sha256_digest(seed_pub),
        inventory,
        streams,
    })
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PubSeedStreamDelta {
    pub path: String,
    pub before_len: u64,
    pub after_len: u64,
    pub before_sha256: Sha256Digest,
    pub after_sha256: Sha256Digest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PubSeedPreservationReport {
    pub schema_version: &'static str,
    pub seed_sha256: Sha256Digest,
    pub output_sha256: Sha256Digest,
    pub container_rewrite_primitive: &'static str,
    pub container_entry_topology_preserved: bool,
    pub allowed_changed_streams: Vec<String>,
    pub changed_streams: Vec<PubSeedStreamDelta>,
    pub preserved_stream_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PubSeededBootstrapReport {
    pub schema_version: &'static str,
    pub structural_materialization_gate: &'static str,
    pub seed: PubSeedManifest,
    pub output: PubSeedManifest,
    pub preservation: PubSeedPreservationReport,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeededStoryPubCandidate {
    pub report: PubSeededBootstrapReport,
    pub story: StoryTextPubCandidate,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SeededStoryPubMaterializationBlocked {
    SeedManifest(PubSeedManifestError),
    Story(StoryTextPubMaterializationBlocked),
    Preservation { detail: String },
}

impl SeededStoryPubMaterializationBlocked {
    pub const fn code(&self) -> &'static str {
        match self {
            Self::SeedManifest(_) => "seed_manifest",
            Self::Story(error) => error.code(),
            Self::Preservation { .. } => "seed_preservation",
        }
    }
}

impl fmt::Display for SeededStoryPubMaterializationBlocked {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SeedManifest(error) => write!(formatter, "seed manifest отклонён: {error}"),
            Self::Story(error) => write!(formatter, "Story materialization отклонён: {error}"),
            Self::Preservation { detail } => {
                write!(formatter, "seed preservation contract нарушен: {detail}")
            }
        }
    }
}

impl std::error::Error for SeededStoryPubMaterializationBlocked {}

impl From<PubSeedManifestError> for SeededStoryPubMaterializationBlocked {
    fn from(value: PubSeedManifestError) -> Self {
        Self::SeedManifest(value)
    }
}

impl From<StoryTextPubMaterializationBlocked> for SeededStoryPubMaterializationBlocked {
    fn from(value: StoryTextPubMaterializationBlocked) -> Self {
        Self::Story(value)
    }
}

/// Bounded bootstrap primitive для будущего New document -> PUB пути.
///
/// Это намеренно не generic new-document writer. Функция фиксирует immutable
/// native seed, применяет только уже доказанную ordinary Story mutation, затем
/// заново строит stream-level manifest и fail-closed проверяет, что вне Quill
/// ничего логически не изменилось.
///
/// Ordinary shape/page structural synthesis остаётся закрыта отдельным gate
/// `STRUCT-WRITER-REPLAY-01`. До его закрытия этот API нельзя трактовать как
/// доказательство создания нового PUB с нуля.
pub fn materialize_seeded_mature_0x2c_story_pub_candidate(
    seed_pub: &[u8],
    request: &StoryTextWriteProbeRequest,
) -> Result<SeededStoryPubCandidate, SeededStoryPubMaterializationBlocked> {
    let seed = inspect_pub_seed_manifest(seed_pub)?;
    let story = materialize_mature_0x2c_story_text_pub_candidate(seed_pub, request)?;
    let output = inspect_pub_seed_manifest(&story.bytes)?;
    let preservation = validate_seed_preservation(&seed, &output, QUILL_STREAM_PATH)?;

    Ok(SeededStoryPubCandidate {
        report: PubSeededBootstrapReport {
            schema_version: PUB_SEEDED_BOOTSTRAP_REPORT_SCHEMA_V0_1,
            structural_materialization_gate: PUB_SEEDED_NEW_DOCUMENT_STRUCTURAL_GATE,
            seed,
            output,
            preservation,
        },
        story,
    })
}

pub fn seeded_bootstrap_report_json(
    report: &PubSeededBootstrapReport,
) -> Result<String, serde_json::Error> {
    serde_json::to_string_pretty(report)
}

fn validate_seed_preservation(
    seed: &PubSeedManifest,
    output: &PubSeedManifest,
    allowed_changed_stream: &str,
) -> Result<PubSeedPreservationReport, SeededStoryPubMaterializationBlocked> {
    let seed_topology = seed
        .inventory
        .entries
        .iter()
        .map(|entry| (entry.path.clone(), entry.kind))
        .collect::<Vec<_>>();
    let output_topology = output
        .inventory
        .entries
        .iter()
        .map(|entry| (entry.path.clone(), entry.kind))
        .collect::<Vec<_>>();
    if seed_topology != output_topology {
        return Err(SeededStoryPubMaterializationBlocked::Preservation {
            detail: "CFB logical entry topology changed".into(),
        });
    }

    let seed_streams = seed
        .streams
        .iter()
        .map(|stream| (stream.path.as_str(), stream))
        .collect::<BTreeMap<_, _>>();
    let output_streams = output
        .streams
        .iter()
        .map(|stream| (stream.path.as_str(), stream))
        .collect::<BTreeMap<_, _>>();

    if seed_streams.len() != output_streams.len() {
        return Err(SeededStoryPubMaterializationBlocked::Preservation {
            detail: format!(
                "logical stream count changed: seed={}, output={}",
                seed_streams.len(),
                output_streams.len()
            ),
        });
    }

    let mut changed_streams = Vec::new();
    let mut preserved_stream_count = 0usize;
    for (path, before) in &seed_streams {
        let after = output_streams.get(path).ok_or_else(|| {
            SeededStoryPubMaterializationBlocked::Preservation {
                detail: format!("output потерял logical stream {path}"),
            }
        })?;

        if before.len == after.len && before.sha256 == after.sha256 {
            preserved_stream_count += 1;
            continue;
        }

        if *path != allowed_changed_stream {
            return Err(SeededStoryPubMaterializationBlocked::Preservation {
                detail: format!(
                    "unexpected logical stream mutation outside writer ownership: {path}"
                ),
            });
        }

        changed_streams.push(PubSeedStreamDelta {
            path: (*path).to_owned(),
            before_len: before.len,
            after_len: after.len,
            before_sha256: before.sha256,
            after_sha256: after.sha256,
        });
    }

    for path in output_streams.keys() {
        if !seed_streams.contains_key(path) {
            return Err(SeededStoryPubMaterializationBlocked::Preservation {
                detail: format!("output добавил unexpected logical stream {path}"),
            });
        }
    }

    if changed_streams.len() != 1 || changed_streams[0].path != allowed_changed_stream {
        return Err(SeededStoryPubMaterializationBlocked::Preservation {
            detail: format!(
                "expected exactly one changed owned stream {allowed_changed_stream}, observed {}",
                changed_streams.len()
            ),
        });
    }

    Ok(PubSeedPreservationReport {
        schema_version: PUB_SEEDED_BOOTSTRAP_REPORT_SCHEMA_V0_1,
        seed_sha256: seed.file_sha256,
        output_sha256: output.file_sha256,
        container_rewrite_primitive: "pub-cfb::replace_stream_reader",
        container_entry_topology_preserved: true,
        allowed_changed_streams: vec![allowed_changed_stream.to_owned()],
        changed_streams,
        preserved_stream_count,
    })
}

fn sha256_digest(bytes: &[u8]) -> Sha256Digest {
    let digest = Sha256::digest(bytes);
    let mut value = [0_u8; 32];
    value.copy_from_slice(&digest);
    Sha256Digest::from_bytes(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::StoryTextWriteProbeRequest;
    use pub_core::StreamPath;
    use pub_quill::parse_confirmed_story_catalog;
    use pub_reader::derive_pub_story_id;

    fn sample3_pub() -> Vec<u8> {
        decode_base64(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../pub-quill/tests/fixtures/Sample3.pub.b64"
        )))
    }

    #[test]
    fn seed_manifest_is_deterministic_and_hashes_every_logical_stream() {
        let source = sample3_pub();
        let first = inspect_pub_seed_manifest(&source).expect("seed manifest");
        let second = inspect_pub_seed_manifest(&source).expect("same seed manifest");

        assert_eq!(first, second);
        assert_eq!(first.file_sha256, sha256_digest(&source));
        let stream_count = first
            .inventory
            .entries
            .iter()
            .filter(|entry| entry.kind == EntryKind::Stream)
            .count();
        assert_eq!(first.streams.len(), stream_count);
        assert!(
            first
                .streams
                .iter()
                .any(|stream| stream.path == QUILL_STREAM_PATH)
        );
    }

    #[test]
    fn seeded_story_candidate_reports_only_owned_quill_delta() {
        let source = sample3_pub();
        let source_hash = sha256_digest(&source);
        let quill = pub_cfb::read_stream_reader(Cursor::new(&source), QUILL_STREAM_PATH)
            .expect("Sample3 Quill stream");
        let catalog = parse_confirmed_story_catalog(StreamPath(QUILL_STREAM_PATH.into()), &quill)
            .expect("Sample3 Story catalog");
        let story = catalog
            .stories
            .iter()
            .find(|story| story.syid.0 == 4)
            .expect("controlled SYID 4");
        let units = story
            .utf16le
            .chunks_exact(2)
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
            .collect::<Vec<_>>();
        let before = String::from_utf16(&units).expect("valid Story text");
        assert!(before.contains("345678"));
        let after = before.replacen("345678", "", 1);

        let request = StoryTextWriteProbeRequest {
            source_hash,
            story_id: derive_pub_story_id(&source_hash, 4).expect("source StoryId"),
            before,
            after,
        };
        let candidate = materialize_seeded_mature_0x2c_story_pub_candidate(&source, &request)
            .expect("seeded Story candidate");

        assert_eq!(candidate.report.seed.file_sha256, source_hash);
        assert_eq!(
            candidate.report.output.file_sha256,
            candidate.story.output_hash
        );
        assert!(
            candidate
                .report
                .preservation
                .container_entry_topology_preserved
        );
        assert_eq!(
            candidate.report.preservation.allowed_changed_streams,
            vec![QUILL_STREAM_PATH.to_owned()]
        );
        assert_eq!(candidate.report.preservation.changed_streams.len(), 1);
        assert_eq!(
            candidate.report.preservation.changed_streams[0].path,
            QUILL_STREAM_PATH
        );
        assert_eq!(
            candidate.report.preservation.preserved_stream_count + 1,
            candidate.report.seed.streams.len()
        );

        let json = seeded_bootstrap_report_json(&candidate.report).expect("report JSON");
        assert!(json.contains(PUB_SEEDED_BOOTSTRAP_REPORT_SCHEMA_V0_1));
        assert!(json.contains(PUB_SEEDED_NEW_DOCUMENT_STRUCTURAL_GATE));
        assert!(json.contains(QUILL_STREAM_PATH));
    }

    fn decode_base64(text: &str) -> Vec<u8> {
        let cleaned = text
            .bytes()
            .filter(|byte| !byte.is_ascii_whitespace())
            .collect::<Vec<_>>();
        assert_eq!(cleaned.len() % 4, 0, "base64 fixture length");

        let mut output = Vec::with_capacity(cleaned.len() / 4 * 3);
        for quartet in cleaned.chunks_exact(4) {
            let a = base64_value(quartet[0]);
            let b = base64_value(quartet[1]);
            let c = if quartet[2] == b'=' {
                0
            } else {
                base64_value(quartet[2])
            };
            let d = if quartet[3] == b'=' {
                0
            } else {
                base64_value(quartet[3])
            };
            output.push((a << 2) | (b >> 4));
            if quartet[2] != b'=' {
                output.push((b << 4) | (c >> 2));
            }
            if quartet[3] != b'=' {
                output.push((c << 6) | d);
            }
        }
        output
    }

    fn base64_value(byte: u8) -> u8 {
        match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            other => panic!("invalid base64 byte {other:#x}"),
        }
    }
}
