//! Source-aware bounded native PUB writer probes и whole-file candidates.
//!
//! Crate отделяет три утверждения:
//! 1. конкретная semantic mutation проходит bounded source-format writer;
//! 2. принятый stream output можно preservation-first материализовать в целый PUB
//!    candidate и повторно открыть текущим reader;
//! 3. native Publisher действительно принимает этот candidate.
//!
//! Здесь реализуются только первые два. Третье остаётся отдельным native-validation gate.

mod seeded;

pub use seeded::{
    PUB_SEEDED_BOOTSTRAP_REPORT_SCHEMA_V0_1, PUB_SEEDED_NEW_DOCUMENT_STRUCTURAL_GATE,
    PubSeedManifest, PubSeedManifestError, PubSeedPreservationReport, PubSeedStreamDelta,
    PubSeedStreamDigest, PubSeededBootstrapReport, SeededStoryPubCandidate,
    SeededStoryPubMaterializationBlocked, inspect_pub_seed_manifest,
    materialize_seeded_mature_0x2c_story_pub_candidate, seeded_bootstrap_report_json,
};

use pub_core::{QuillSyid, StreamPath};
use pub_model::{Sha256Digest, StoryId};
use pub_quill::{
    QuillStoryTextEdit, QuillStoryTextWritePlan, QuillStoryWriteError,
    parse_confirmed_story_catalog, plan_quill_story_text_edit,
};
use pub_reader::{
    QUILL_STREAM_PATH, build_mature_0x2c_source_graph, derive_pub_story_id,
    resolve_pub_source_graph,
};
use sha2::{Digest, Sha256};
use std::fmt;
use std::io::Cursor;

pub const PUB_WRITER_PROBE_VERSION_V0_1: &str = "pub-writer-probe-v0.1";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoryTextWriteProbeRequest {
    pub source_hash: Sha256Digest,
    pub story_id: StoryId,
    pub before: String,
    pub after: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoryTextWriteProbeAccepted {
    pub story_id: StoryId,
    pub story_syid: QuillSyid,
    pub edit: QuillStoryTextEdit,
    pub plan: QuillStoryTextWritePlan,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StoryTextWriteProbeBlocked {
    SourceHashMismatch {
        expected: Sha256Digest,
        actual: Sha256Digest,
    },
    CfbRead {
        detail: String,
    },
    CatalogRead {
        detail: String,
    },
    StoryIdentityDerivation {
        detail: String,
    },
    StoryNotFound {
        story_id: StoryId,
    },
    StoryIdentityAmbiguous {
        story_id: StoryId,
    },
    BeforeTextMismatch {
        story_id: StoryId,
    },
    NoChange {
        story_id: StoryId,
    },
    Utf16IndexOverflow,
    ReplacementSliceInvalidUtf16,
    Writer(QuillStoryWriteError),
}

impl StoryTextWriteProbeBlocked {
    pub const fn code(&self) -> &'static str {
        match self {
            Self::SourceHashMismatch { .. } => "source_hash_mismatch",
            Self::CfbRead { .. } => "cfb_read",
            Self::CatalogRead { .. } => "quill_catalog_read",
            Self::StoryIdentityDerivation { .. } => "story_identity_derivation",
            Self::StoryNotFound { .. } => "story_not_found",
            Self::StoryIdentityAmbiguous { .. } => "story_identity_ambiguous",
            Self::BeforeTextMismatch { .. } => "before_text_mismatch",
            Self::NoChange { .. } => "no_change",
            Self::Utf16IndexOverflow => "utf16_index_overflow",
            Self::ReplacementSliceInvalidUtf16 => "replacement_slice_invalid_utf16",
            Self::Writer(error) => quill_writer_error_code(error),
        }
    }
}

impl fmt::Display for StoryTextWriteProbeBlocked {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SourceHashMismatch { expected, actual } => write!(
                formatter,
                "SHA-256 исходных PUB-байтов не совпадает: ожидался {expected}, получен {actual}"
            ),
            Self::CfbRead { detail } => {
                write!(
                    formatter,
                    "не удалось прочитать Quill stream из CFB: {detail}"
                )
            }
            Self::CatalogRead { detail } => {
                write!(
                    formatter,
                    "не удалось разобрать Quill Story catalog: {detail}"
                )
            }
            Self::StoryIdentityDerivation { detail } => {
                write!(formatter, "не удалось вывести canonical StoryId: {detail}")
            }
            Self::StoryNotFound { story_id } => {
                write!(formatter, "Story {story_id:?} не найдена в исходном Quill")
            }
            Self::StoryIdentityAmbiguous { story_id } => {
                write!(
                    formatter,
                    "Story {story_id:?} неоднозначна в исходном Quill"
                )
            }
            Self::BeforeTextMismatch { story_id } => write!(
                formatter,
                "текущий текст Story {story_id:?} не совпадает с before-текстом mutation"
            ),
            Self::NoChange { story_id } => {
                write!(formatter, "mutation Story {story_id:?} не меняет текст")
            }
            Self::Utf16IndexOverflow => {
                formatter.write_str("UTF-16 индекс mutation не помещается в u32")
            }
            Self::ReplacementSliceInvalidUtf16 => {
                formatter.write_str("минимальный replacement slice не является валидным UTF-16")
            }
            Self::Writer(error) => {
                write!(formatter, "bounded Quill writer отклонил mutation: {error}")
            }
        }
    }
}

impl std::error::Error for StoryTextWriteProbeBlocked {}

/// Проверяет конкретную full-Story text mutation на точных исходных PUB-байтах.
///
/// Успех доказывает только готовность bounded Quill stream writer для этой
/// mutation. Whole-file CFB materialization и post-write validation остаются
/// отдельными обязательными gates.
pub fn probe_mature_0x2c_story_text_write(
    source_pub: &[u8],
    request: &StoryTextWriteProbeRequest,
) -> Result<StoryTextWriteProbeAccepted, StoryTextWriteProbeBlocked> {
    let actual_hash = sha256_digest(source_pub);
    if actual_hash != request.source_hash {
        return Err(StoryTextWriteProbeBlocked::SourceHashMismatch {
            expected: request.source_hash,
            actual: actual_hash,
        });
    }

    let quill = pub_cfb::read_stream_reader(Cursor::new(source_pub), QUILL_STREAM_PATH).map_err(
        |error| StoryTextWriteProbeBlocked::CfbRead {
            detail: format!("{error:#}"),
        },
    )?;
    let stream = StreamPath(QUILL_STREAM_PATH.into());
    let catalog = parse_confirmed_story_catalog(stream.clone(), &quill).map_err(|error| {
        StoryTextWriteProbeBlocked::CatalogRead {
            detail: error.to_string(),
        }
    })?;

    let mut matched = None;
    for story in &catalog.stories {
        let derived = derive_pub_story_id(&request.source_hash, story.syid.0).map_err(|error| {
            StoryTextWriteProbeBlocked::StoryIdentityDerivation {
                detail: format!("{error:#}"),
            }
        })?;
        if derived != request.story_id {
            continue;
        }
        if matched.is_some() {
            return Err(StoryTextWriteProbeBlocked::StoryIdentityAmbiguous {
                story_id: request.story_id,
            });
        }
        matched = Some(story);
    }

    let story = matched.ok_or(StoryTextWriteProbeBlocked::StoryNotFound {
        story_id: request.story_id,
    })?;
    let source_units = utf16le_units(&story.utf16le)
        .map_err(|detail| StoryTextWriteProbeBlocked::CatalogRead { detail })?;
    let before_units = request.before.encode_utf16().collect::<Vec<_>>();
    if source_units != before_units {
        return Err(StoryTextWriteProbeBlocked::BeforeTextMismatch {
            story_id: request.story_id,
        });
    }

    let after_units = request.after.encode_utf16().collect::<Vec<_>>();
    let diff = minimal_utf16_edit(&before_units, &after_units).ok_or(
        StoryTextWriteProbeBlocked::NoChange {
            story_id: request.story_id,
        },
    )?;
    let replacement =
        String::from_utf16(&after_units[diff.replacement_start..diff.replacement_end])
            .map_err(|_| StoryTextWriteProbeBlocked::ReplacementSliceInvalidUtf16)?;
    let start_utf16 =
        u32::try_from(diff.start).map_err(|_| StoryTextWriteProbeBlocked::Utf16IndexOverflow)?;
    let delete_utf16 = u32::try_from(diff.delete_len)
        .map_err(|_| StoryTextWriteProbeBlocked::Utf16IndexOverflow)?;

    let edit = QuillStoryTextEdit {
        story_syid: story.syid,
        start_utf16,
        delete_utf16,
        replacement,
    };
    let plan = plan_quill_story_text_edit(stream, &quill, &edit)
        .map_err(StoryTextWriteProbeBlocked::Writer)?;

    Ok(StoryTextWriteProbeAccepted {
        story_id: request.story_id,
        story_syid: story.syid,
        edit,
        plan,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoryTextPubCandidate {
    pub source_hash: Sha256Digest,
    pub output_hash: Sha256Digest,
    pub source_story_id: StoryId,
    pub output_story_id: StoryId,
    pub story_syid: QuillSyid,
    pub quill_plan: QuillStoryTextWritePlan,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StoryTextPubMaterializationBlocked {
    Probe(StoryTextWriteProbeBlocked),
    CfbMaterialization {
        detail: String,
    },
    ReopenSourceGraph {
        detail: String,
    },
    ReopenResolve {
        detail: String,
    },
    OutputStoryIdentity {
        detail: String,
    },
    OutputStoryMissing {
        story_id: StoryId,
    },
    OutputStoryTextMismatch {
        story_id: StoryId,
        expected: String,
        actual: String,
    },
}

impl StoryTextPubMaterializationBlocked {
    pub const fn code(&self) -> &'static str {
        match self {
            Self::Probe(error) => error.code(),
            Self::CfbMaterialization { .. } => "cfb_materialization",
            Self::ReopenSourceGraph { .. } => "reopen_source_graph",
            Self::ReopenResolve { .. } => "reopen_resolve",
            Self::OutputStoryIdentity { .. } => "output_story_identity",
            Self::OutputStoryMissing { .. } => "output_story_missing",
            Self::OutputStoryTextMismatch { .. } => "output_story_text_mismatch",
        }
    }
}

impl fmt::Display for StoryTextPubMaterializationBlocked {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Probe(error) => {
                write!(formatter, "Story writer probe отклонил mutation: {error}")
            }
            Self::CfbMaterialization { detail } => {
                write!(
                    formatter,
                    "не удалось materialize whole-file CFB candidate: {detail}"
                )
            }
            Self::ReopenSourceGraph { detail } => {
                write!(
                    formatter,
                    "output PUB не прошёл повторный SourceGraph parse: {detail}"
                )
            }
            Self::ReopenResolve { detail } => {
                write!(
                    formatter,
                    "output PUB не прошёл повторный semantic resolve: {detail}"
                )
            }
            Self::OutputStoryIdentity { detail } => {
                write!(formatter, "не удалось вывести StoryId output PUB: {detail}")
            }
            Self::OutputStoryMissing { story_id } => {
                write!(
                    formatter,
                    "output PUB не содержит ожидаемую Story {story_id:?}"
                )
            }
            Self::OutputStoryTextMismatch {
                story_id,
                expected,
                actual,
            } => write!(
                formatter,
                "output PUB Story {story_id:?} имеет другой текст: ожидалось {expected:?}, получено {actual:?}"
            ),
        }
    }
}

impl std::error::Error for StoryTextPubMaterializationBlocked {}

impl From<StoryTextWriteProbeBlocked> for StoryTextPubMaterializationBlocked {
    fn from(value: StoryTextWriteProbeBlocked) -> Self {
        Self::Probe(value)
    }
}

/// Строит whole-file PUB candidate для одной уже доказанной ordinary Story mutation.
///
/// Pipeline:
/// source PUB -> bounded Quill plan -> in-copy CFB stream mutation Quill
/// -> preservation validation untouched logical streams/metadata -> current reader reopen
/// -> semantic check.
///
/// Успех этой функции не является native Publisher acceptance proof и сам по
/// себе не должен включать product action Save PUB.
pub fn materialize_mature_0x2c_story_text_pub_candidate(
    source_pub: &[u8],
    request: &StoryTextWriteProbeRequest,
) -> Result<StoryTextPubCandidate, StoryTextPubMaterializationBlocked> {
    let accepted = probe_mature_0x2c_story_text_write(source_pub, request)?;

    let output = pub_cfb::replace_stream_reader(
        Cursor::new(source_pub),
        QUILL_STREAM_PATH,
        &accepted.plan.output_stream,
    )
    .map_err(
        |error| StoryTextPubMaterializationBlocked::CfbMaterialization {
            detail: format!("{error:#}"),
        },
    )?;

    let output_hash = sha256_digest(&output);
    let reopened =
        build_mature_0x2c_source_graph(Cursor::new(&output), output_hash).map_err(|error| {
            StoryTextPubMaterializationBlocked::ReopenSourceGraph {
                detail: format!("{error:#}"),
            }
        })?;
    let resolved = resolve_pub_source_graph(&reopened.graph).map_err(|error| {
        StoryTextPubMaterializationBlocked::ReopenResolve {
            detail: format!("{error:#}"),
        }
    })?;

    let output_story_id =
        derive_pub_story_id(&output_hash, accepted.story_syid.0).map_err(|error| {
            StoryTextPubMaterializationBlocked::OutputStoryIdentity {
                detail: format!("{error:#}"),
            }
        })?;
    let output_story = resolved.graph.stories.get(&output_story_id).ok_or(
        StoryTextPubMaterializationBlocked::OutputStoryMissing {
            story_id: output_story_id,
        },
    )?;
    if output_story.text != request.after {
        return Err(
            StoryTextPubMaterializationBlocked::OutputStoryTextMismatch {
                story_id: output_story_id,
                expected: request.after.clone(),
                actual: output_story.text.clone(),
            },
        );
    }

    Ok(StoryTextPubCandidate {
        source_hash: request.source_hash,
        output_hash,
        source_story_id: request.story_id,
        output_story_id,
        story_syid: accepted.story_syid,
        quill_plan: accepted.plan,
        bytes: output,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Utf16EditRange {
    start: usize,
    delete_len: usize,
    replacement_start: usize,
    replacement_end: usize,
}

fn minimal_utf16_edit(before: &[u16], after: &[u16]) -> Option<Utf16EditRange> {
    if before == after {
        return None;
    }

    let mut prefix = 0;
    let shared = before.len().min(after.len());
    while prefix < shared && before[prefix] == after[prefix] {
        prefix += 1;
    }
    while boundary_splits_surrogate(before, prefix) || boundary_splits_surrogate(after, prefix) {
        prefix = prefix.saturating_sub(1);
    }

    let mut suffix = 0;
    while suffix < before.len().saturating_sub(prefix)
        && suffix < after.len().saturating_sub(prefix)
        && before[before.len() - 1 - suffix] == after[after.len() - 1 - suffix]
    {
        suffix += 1;
    }
    while suffix > 0 {
        let before_boundary = before.len() - suffix;
        let after_boundary = after.len() - suffix;
        if boundary_splits_surrogate(before, before_boundary)
            || boundary_splits_surrogate(after, after_boundary)
        {
            suffix -= 1;
        } else {
            break;
        }
    }

    let before_end = before.len() - suffix;
    let after_end = after.len() - suffix;
    Some(Utf16EditRange {
        start: prefix,
        delete_len: before_end - prefix,
        replacement_start: prefix,
        replacement_end: after_end,
    })
}

fn boundary_splits_surrogate(units: &[u16], boundary: usize) -> bool {
    boundary > 0
        && boundary < units.len()
        && (0xd800..=0xdbff).contains(&units[boundary - 1])
        && (0xdc00..=0xdfff).contains(&units[boundary])
}

fn utf16le_units(bytes: &[u8]) -> Result<Vec<u16>, String> {
    if bytes.len() % 2 != 0 {
        return Err("Quill Story содержит нечётное число UTF-16LE байтов".into());
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

const fn quill_writer_error_code(error: &QuillStoryWriteError) -> &'static str {
    match error {
        QuillStoryWriteError::Read(_) => "quill_read",
        QuillStoryWriteError::StoryNotFound(_) => "quill_story_not_found",
        QuillStoryWriteError::DuplicateStoryIdentity(_) => "quill_story_identity_ambiguous",
        QuillStoryWriteError::TargetIsTableStory(_) => "quill_table_story",
        QuillStoryWriteError::TargetHasTokenLayer(_) => "quill_token_layer",
        QuillStoryWriteError::EditOutOfBounds => "quill_edit_out_of_bounds",
        QuillStoryWriteError::NonBmpSourceStory => "quill_non_bmp_source",
        QuillStoryWriteError::NonBmpReplacement => "quill_non_bmp_replacement",
        QuillStoryWriteError::ParagraphMarkMutation => "quill_paragraph_mark_mutation",
        QuillStoryWriteError::LengthPreservingEditOutOfScope => {
            "quill_length_preserving_out_of_scope"
        }
        QuillStoryWriteError::MissingBoundaryChunk(_) => "quill_missing_fd_boundary",
        QuillStoryWriteError::MalformedBoundaryChunk(_) => "quill_malformed_fd_boundary",
        QuillStoryWriteError::BoundaryOutsideText(_, _) => "quill_fd_boundary_outside_text",
        QuillStoryWriteError::InsertionAtExistingFdBoundary => "quill_insertion_at_fd_boundary",
        QuillStoryWriteError::EditTouchesExistingFdBoundary(_) => "quill_edit_touches_fd_boundary",
        QuillStoryWriteError::EditCrossesExistingFdBoundary(_) => "quill_edit_crosses_fd_boundary",
        QuillStoryWriteError::InsertionAtExistingBteBoundary(_) => {
            "quill_insertion_at_bte_boundary"
        }
        QuillStoryWriteError::EditTouchesExistingBteBoundary(_) => {
            "quill_edit_touches_bte_boundary"
        }
        QuillStoryWriteError::EditCrossesExistingBteBoundary(_) => {
            "quill_edit_crosses_bte_boundary"
        }
        QuillStoryWriteError::MissingAlignedServiceAnchor => "quill_missing_service_anchor",
        QuillStoryWriteError::InsufficientCapacity => "quill_insufficient_capacity",
        QuillStoryWriteError::PositiveGrowthWouldDiscardNonZeroUnknownAnchorBytes => {
            "quill_growth_would_discard_unknown"
        }
        QuillStoryWriteError::PositiveGrowthWouldDiscardDescriptorPayload(_) => {
            "quill_growth_would_discard_descriptor"
        }
        QuillStoryWriteError::DescriptorMetadataInsideRelocationWindow => {
            "quill_descriptor_metadata_in_relocation_window"
        }
        QuillStoryWriteError::UnsupportedBteDataSize(_, _) => "quill_unsupported_bte_data_size",
        QuillStoryWriteError::MalformedBteChunk(_) => "quill_malformed_bte",
        QuillStoryWriteError::SyidHeaderOverflow => "quill_syid_header_overflow",
        QuillStoryWriteError::IntegerOverflow => "quill_integer_overflow",
        QuillStoryWriteError::OutputValidationFailed(_) => "quill_output_validation_failed",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample3_pub() -> Vec<u8> {
        decode_base64(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../pub-quill/tests/fixtures/Sample3.pub.b64"
        )))
    }

    #[test]
    fn accepted_story_materializes_to_reopenable_whole_pub_candidate() {
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
        let before = String::from_utf16(&utf16le_units(&story.utf16le).expect("valid UTF-16"))
            .expect("valid Story text");
        assert!(before.contains("345678"), "controlled marker must exist");
        let after = before.replacen("345678", "", 1);
        let request = StoryTextWriteProbeRequest {
            source_hash,
            story_id: derive_pub_story_id(&source_hash, 4).expect("source StoryId"),
            before,
            after: after.clone(),
        };

        let source_before = source.clone();
        let candidate = materialize_mature_0x2c_story_text_pub_candidate(&source, &request)
            .expect("whole-file candidate should materialize");

        assert_eq!(source, source_before, "source bytes must remain immutable");
        assert_ne!(candidate.output_hash, source_hash);
        assert_eq!(candidate.story_syid.0, 4);
        assert_eq!(
            pub_cfb::read_stream_reader(Cursor::new(&candidate.bytes), QUILL_STREAM_PATH)
                .expect("output Quill stream"),
            candidate.quill_plan.output_stream
        );

        let reopened =
            build_mature_0x2c_source_graph(Cursor::new(&candidate.bytes), candidate.output_hash)
                .expect("candidate SourceGraph");
        let resolved = resolve_pub_source_graph(&reopened.graph).expect("candidate resolve");
        assert_eq!(
            resolved.graph.stories[&candidate.output_story_id].text,
            after
        );
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

    #[test]
    fn minimal_utf16_edit_keeps_only_changed_middle() {
        let before = "abcXYZdef".encode_utf16().collect::<Vec<_>>();
        let after = "abcQdef".encode_utf16().collect::<Vec<_>>();
        let diff = minimal_utf16_edit(&before, &after).expect("есть изменение");

        assert_eq!(diff.start, 3);
        assert_eq!(diff.delete_len, 3);
        assert_eq!(
            String::from_utf16(&after[diff.replacement_start..diff.replacement_end]).unwrap(),
            "Q"
        );
    }

    #[test]
    fn minimal_utf16_edit_does_not_split_surrogate_pair() {
        let before = "A😀B".encode_utf16().collect::<Vec<_>>();
        let after = "A😃B".encode_utf16().collect::<Vec<_>>();
        let diff = minimal_utf16_edit(&before, &after).expect("есть изменение");

        assert_eq!(diff.start, 1);
        assert_eq!(diff.delete_len, 2);
        assert_eq!(
            String::from_utf16(&after[diff.replacement_start..diff.replacement_end]).unwrap(),
            "😃"
        );
    }
}
