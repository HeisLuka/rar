#![forbid(unsafe_code)]

use serde::{Deserialize, Serialize};

pub const RESUME_SCHEMA_V1: &str = "chaptera.reader-resume.v1";
pub const MIN_ZOOM_MILLI: u32 = 100;
pub const MAX_ZOOM_MILLI: u32 = 4_000;
const MAX_SOURCE_REFERENCE_BYTES: usize = 32_768;
const MAX_WRITER_SESSION_ID_BYTES: usize = 128;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResumeSourceIdentityV1 {
    pub source_reference: String,
    pub sha256_hex: String,
    pub byte_len: u64,
}

impl ResumeSourceIdentityV1 {
    pub fn validate(&self) -> Result<(), ResumeError> {
        if self.source_reference.is_empty()
            || self.source_reference.len() > MAX_SOURCE_REFERENCE_BYTES
            || self.source_reference.contains('\0')
        {
            return Err(ResumeError::InvalidSourceReference);
        }
        if self.sha256_hex.len() != 64
            || !self
                .sha256_hex
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(ResumeError::InvalidSourceSha256);
        }
        Ok(())
    }

    pub fn matches_observed(
        &self,
        source_reference: &str,
        sha256_hex: &str,
        byte_len: u64,
    ) -> bool {
        self.source_reference == source_reference
            && self.sha256_hex == sha256_hex
            && self.byte_len == byte_len
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReadingStateV1 {
    pub page_index: u32,
    pub viewport_x_emu: i64,
    pub viewport_y_emu: i64,
    pub zoom_milli: u32,
}

impl ReadingStateV1 {
    pub fn validate(&self) -> Result<(), ResumeError> {
        if !(MIN_ZOOM_MILLI..=MAX_ZOOM_MILLI).contains(&self.zoom_milli) {
            return Err(ResumeError::InvalidZoom);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResumeCheckpointV1 {
    pub writer_session_id: String,
    pub source: ResumeSourceIdentityV1,
    pub reading: ReadingStateV1,
}

impl ResumeCheckpointV1 {
    pub fn validate(&self) -> Result<(), ResumeError> {
        if self.writer_session_id.is_empty()
            || self.writer_session_id.len() > MAX_WRITER_SESSION_ID_BYTES
        {
            return Err(ResumeError::InvalidWriterSessionId);
        }
        self.source.validate()?;
        self.reading.validate()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResumeStoreV1 {
    pub schema_version: String,
    pub revision: u64,
    pub entry: Option<ResumeCheckpointV1>,
}

impl Default for ResumeStoreV1 {
    fn default() -> Self {
        Self {
            schema_version: RESUME_SCHEMA_V1.to_owned(),
            revision: 0,
            entry: None,
        }
    }
}

impl ResumeStoreV1 {
    pub fn validate(&self) -> Result<(), ResumeError> {
        if self.schema_version != RESUME_SCHEMA_V1 {
            return Err(ResumeError::SchemaMismatch);
        }
        if let Some(entry) = &self.entry {
            entry.validate()?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResumeMutationV1 {
    OpenDocument(ResumeCheckpointV1),
    ReadingCheckpoint(ResumeCheckpointV1),
    Clear,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResumeError {
    SchemaMismatch,
    InvalidSourceReference,
    InvalidSourceSha256,
    InvalidWriterSessionId,
    InvalidZoom,
    RevisionOverflow,
    StaleRevision { expected: u64, actual: u64 },
    MissingOpenDocument,
    SourceChangedWithoutOpen,
    WriterSessionMismatch,
    InvalidJson,
}

pub fn apply_mutation(
    current: &ResumeStoreV1,
    expected_revision: u64,
    mutation: ResumeMutationV1,
) -> Result<ResumeStoreV1, ResumeError> {
    current.validate()?;
    if current.revision != expected_revision {
        return Err(ResumeError::StaleRevision {
            expected: expected_revision,
            actual: current.revision,
        });
    }

    let revision = current
        .revision
        .checked_add(1)
        .ok_or(ResumeError::RevisionOverflow)?;

    let entry = match mutation {
        ResumeMutationV1::OpenDocument(checkpoint) => {
            checkpoint.validate()?;
            Some(checkpoint)
        }
        ResumeMutationV1::ReadingCheckpoint(checkpoint) => {
            checkpoint.validate()?;
            let committed = current
                .entry
                .as_ref()
                .ok_or(ResumeError::MissingOpenDocument)?;
            if checkpoint.source != committed.source {
                return Err(ResumeError::SourceChangedWithoutOpen);
            }
            if checkpoint.writer_session_id != committed.writer_session_id {
                return Err(ResumeError::WriterSessionMismatch);
            }
            Some(checkpoint)
        }
        ResumeMutationV1::Clear => None,
    };

    Ok(ResumeStoreV1 {
        schema_version: RESUME_SCHEMA_V1.to_owned(),
        revision,
        entry,
    })
}

pub fn encode_store(store: &ResumeStoreV1) -> Result<Vec<u8>, ResumeError> {
    store.validate()?;
    serde_json::to_vec_pretty(store).map_err(|_| ResumeError::InvalidJson)
}

pub fn decode_store(bytes: &[u8]) -> Result<ResumeStoreV1, ResumeError> {
    let store: ResumeStoreV1 =
        serde_json::from_slice(bytes).map_err(|_| ResumeError::InvalidJson)?;
    store.validate()?;
    Ok(store)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source(reference: &str, marker: char) -> ResumeSourceIdentityV1 {
        ResumeSourceIdentityV1 {
            source_reference: reference.to_owned(),
            sha256_hex: std::iter::repeat_n(marker, 64).collect(),
            byte_len: 123_456,
        }
    }

    fn checkpoint(
        writer_session_id: &str,
        source: ResumeSourceIdentityV1,
        page_index: u32,
    ) -> ResumeCheckpointV1 {
        ResumeCheckpointV1 {
            writer_session_id: writer_session_id.to_owned(),
            source,
            reading: ReadingStateV1 {
                page_index,
                viewport_x_emu: 1_000,
                viewport_y_emu: 2_000,
                zoom_milli: 1_250,
            },
        }
    }

    #[test]
    fn open_then_reading_checkpoint_advances_monotonic_revision() {
        let empty = ResumeStoreV1::default();
        let opened = apply_mutation(
            &empty,
            0,
            ResumeMutationV1::OpenDocument(checkpoint(
                "session-a",
                source("C:\\docs\\a.pub", 'a'),
                0,
            )),
        )
        .expect("open");
        assert_eq!(opened.revision, 1);

        let reading = apply_mutation(
            &opened,
            1,
            ResumeMutationV1::ReadingCheckpoint(checkpoint(
                "session-a",
                source("C:\\docs\\a.pub", 'a'),
                4,
            )),
        )
        .expect("reading checkpoint");
        assert_eq!(reading.revision, 2);
        assert_eq!(reading.entry.expect("entry").reading.page_index, 4);
    }

    #[test]
    fn stale_process_cannot_overwrite_newer_open() {
        let empty = ResumeStoreV1::default();
        let a = apply_mutation(
            &empty,
            0,
            ResumeMutationV1::OpenDocument(checkpoint(
                "session-a",
                source("C:\\docs\\a.pub", 'a'),
                1,
            )),
        )
        .expect("open A");
        let b = apply_mutation(
            &a,
            1,
            ResumeMutationV1::OpenDocument(checkpoint(
                "session-b",
                source("C:\\docs\\b.pub", 'b'),
                0,
            )),
        )
        .expect("open B");

        let stale = apply_mutation(
            &b,
            1,
            ResumeMutationV1::ReadingCheckpoint(checkpoint(
                "session-a",
                source("C:\\docs\\a.pub", 'a'),
                9,
            )),
        );
        assert_eq!(
            stale,
            Err(ResumeError::StaleRevision {
                expected: 1,
                actual: 2,
            })
        );
    }

    #[test]
    fn old_writer_cannot_adopt_newer_same_source_session() {
        let empty = ResumeStoreV1::default();
        let a = apply_mutation(
            &empty,
            0,
            ResumeMutationV1::OpenDocument(checkpoint(
                "session-a",
                source("C:\\docs\\a.pub", 'a'),
                1,
            )),
        )
        .expect("open A");
        let newer = apply_mutation(
            &a,
            1,
            ResumeMutationV1::OpenDocument(checkpoint(
                "session-b",
                source("C:\\docs\\a.pub", 'a'),
                2,
            )),
        )
        .expect("newer process opens same source");

        let stale_writer = apply_mutation(
            &newer,
            2,
            ResumeMutationV1::ReadingCheckpoint(checkpoint(
                "session-a",
                source("C:\\docs\\a.pub", 'a'),
                8,
            )),
        );
        assert_eq!(stale_writer, Err(ResumeError::WriterSessionMismatch));
    }

    #[test]
    fn reading_checkpoint_cannot_switch_source_identity() {
        let empty = ResumeStoreV1::default();
        let opened = apply_mutation(
            &empty,
            0,
            ResumeMutationV1::OpenDocument(checkpoint(
                "session-a",
                source("C:\\docs\\a.pub", 'a'),
                0,
            )),
        )
        .expect("open");

        let changed = apply_mutation(
            &opened,
            1,
            ResumeMutationV1::ReadingCheckpoint(checkpoint(
                "session-a",
                source("C:\\docs\\a.pub", 'b'),
                1,
            )),
        );
        assert_eq!(changed, Err(ResumeError::SourceChangedWithoutOpen));
    }

    #[test]
    fn clear_is_a_revisioned_tombstone_not_revision_reset() {
        let empty = ResumeStoreV1::default();
        let opened = apply_mutation(
            &empty,
            0,
            ResumeMutationV1::OpenDocument(checkpoint(
                "session-a",
                source("C:\\docs\\a.pub", 'a'),
                0,
            )),
        )
        .expect("open");
        let cleared = apply_mutation(&opened, 1, ResumeMutationV1::Clear).expect("clear");
        assert_eq!(cleared.revision, 2);
        assert!(cleared.entry.is_none());

        let stale = apply_mutation(
            &cleared,
            1,
            ResumeMutationV1::ReadingCheckpoint(checkpoint(
                "session-a",
                source("C:\\docs\\a.pub", 'a'),
                7,
            )),
        );
        assert!(matches!(stale, Err(ResumeError::StaleRevision { .. })));
    }

    #[test]
    fn observed_source_must_match_reference_hash_and_length() {
        let source = source("C:\\docs\\a.pub", 'a');
        assert!(source.matches_observed(
            "C:\\docs\\a.pub",
            &"a".repeat(64),
            123_456
        ));
        assert!(!source.matches_observed(
            "C:\\docs\\a.pub",
            &"b".repeat(64),
            123_456
        ));
        assert!(!source.matches_observed(
            "C:\\moved\\a.pub",
            &"a".repeat(64),
            123_456
        ));
    }

    #[test]
    fn corrupt_truncated_and_unknown_json_fail_closed() {
        assert_eq!(decode_store(br#"{"schema_version":"#), Err(ResumeError::InvalidJson));

        let unknown = br#"{
          "schema_version":"chaptera.reader-resume.v1",
          "revision":0,
          "entry":null,
          "cached_document_text":"must-not-be-admitted"
        }"#;
        assert_eq!(decode_store(unknown), Err(ResumeError::InvalidJson));
    }

    #[test]
    fn encoded_state_roundtrips_without_document_content() {
        let opened = apply_mutation(
            &ResumeStoreV1::default(),
            0,
            ResumeMutationV1::OpenDocument(checkpoint(
                "session-a",
                source("C:\\docs\\a.pub", 'a'),
                3,
            )),
        )
        .expect("open");

        let bytes = encode_store(&opened).expect("encode");
        let text = std::str::from_utf8(&bytes).expect("utf8");
        assert!(!text.contains("recovered_text"));
        assert!(!text.contains("render_plan"));
        assert_eq!(decode_store(&bytes).expect("decode"), opened);
    }

    #[test]
    fn invalid_zoom_and_noncanonical_hash_are_rejected() {
        let mut invalid = checkpoint(
            "session-a",
            source("C:\\docs\\a.pub", 'a'),
            0,
        );
        invalid.reading.zoom_milli = 99;
        assert_eq!(invalid.validate(), Err(ResumeError::InvalidZoom));

        invalid.reading.zoom_milli = 1_000;
        invalid.source.sha256_hex = "A".repeat(64);
        assert_eq!(invalid.validate(), Err(ResumeError::InvalidSourceSha256));
    }
}
