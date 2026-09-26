use pub_editor::{EditorSession, StoryId};
use pub_model::{AuthorityClass, ReadConfidence, SourceRole};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::fmt;

pub const STORY_EDIT_DOMAIN_VERSION_V1: &str = "chaptera.story-edit-domain.v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StoryProvenanceV1 {
    ChapteraCreated,
    ImportedMatureQuillTerminalCr,
    ImportedUnknown,
}

impl StoryProvenanceV1 {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ChapteraCreated => "chaptera_created",
            Self::ImportedMatureQuillTerminalCr => "imported_mature_quill_terminal_cr",
            Self::ImportedUnknown => "imported_unknown",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoryProvenanceHintV1 {
    /// Explicit authority from the Chaptera-created Story lifecycle.
    ChapteraCreated,
    /// Derive only from persisted source carrier/profile evidence.
    ImportedAuto,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProtectedStoryRangeV1 {
    pub start_scalar: u32,
    pub end_scalar: u32,
    pub reason: String,
    pub provenance: StoryProvenanceV1,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoryEditDomainV1 {
    pub protocol_version: String,
    pub story_id: String,
    pub provenance: StoryProvenanceV1,
    pub status: String,
    pub raw_scalar_len: u32,
    pub editable_start_scalar: Option<u32>,
    pub editable_end_scalar: Option<u32>,
    pub caret_start_boundary: Option<u32>,
    pub caret_end_boundary: Option<u32>,
    pub protected_ranges: Vec<ProtectedStoryRangeV1>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoryEditDomainError {
    pub code: &'static str,
    pub message: String,
}

impl StoryEditDomainError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl fmt::Display for StoryEditDomainError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for StoryEditDomainError {}

fn scalar_len(text: &str) -> Result<u32, StoryEditDomainError> {
    u32::try_from(text.chars().count())
        .map_err(|_| StoryEditDomainError::new("invalid_story", "Story scalar length overflows u32"))
}

pub fn derive_story_edit_domain_v1(
    story_id: impl Into<String>,
    story_text: &str,
    provenance: StoryProvenanceV1,
) -> Result<StoryEditDomainV1, StoryEditDomainError> {
    let story_id = story_id.into();
    if story_id.is_empty() {
        return Err(StoryEditDomainError::new(
            "invalid_story",
            "story_id is required",
        ));
    }
    let raw_scalar_len = scalar_len(story_text)?;

    match provenance {
        StoryProvenanceV1::ChapteraCreated => Ok(StoryEditDomainV1 {
            protocol_version: STORY_EDIT_DOMAIN_VERSION_V1.to_owned(),
            story_id,
            provenance,
            status: "known".to_owned(),
            raw_scalar_len,
            editable_start_scalar: Some(0),
            editable_end_scalar: Some(raw_scalar_len),
            caret_start_boundary: Some(0),
            caret_end_boundary: Some(raw_scalar_len),
            protected_ranges: Vec::new(),
        }),
        StoryProvenanceV1::ImportedMatureQuillTerminalCr => {
            if raw_scalar_len == 0 || !story_text.ends_with('\r') {
                return Err(StoryEditDomainError::new(
                    "invalid_provenance",
                    "proven terminal-CR provenance requires final U+000D scalar",
                ));
            }
            let protected_start = raw_scalar_len - 1;
            Ok(StoryEditDomainV1 {
                protocol_version: STORY_EDIT_DOMAIN_VERSION_V1.to_owned(),
                story_id,
                provenance,
                status: "known".to_owned(),
                raw_scalar_len,
                editable_start_scalar: Some(0),
                editable_end_scalar: Some(protected_start),
                caret_start_boundary: Some(0),
                caret_end_boundary: Some(protected_start),
                protected_ranges: vec![ProtectedStoryRangeV1 {
                    start_scalar: protected_start,
                    end_scalar: raw_scalar_len,
                    reason: "source_terminal_paragraph_mark".to_owned(),
                    provenance,
                }],
            })
        }
        StoryProvenanceV1::ImportedUnknown => Ok(StoryEditDomainV1 {
            protocol_version: STORY_EDIT_DOMAIN_VERSION_V1.to_owned(),
            story_id,
            provenance,
            status: "edit_domain_unknown".to_owned(),
            raw_scalar_len,
            editable_start_scalar: None,
            editable_end_scalar: None,
            caret_start_boundary: None,
            caret_end_boundary: None,
            protected_ranges: Vec::new(),
        }),
    }
}

pub fn validate_ordinary_story_range_v1(
    domain: &StoryEditDomainV1,
    start_scalar: u32,
    end_scalar: u32,
) -> Result<(), StoryEditDomainError> {
    if domain.status != "known" {
        return Err(StoryEditDomainError::new(
            "edit_domain_unknown",
            "ordinary edit domain is unknown for imported Story provenance",
        ));
    }
    if end_scalar < start_scalar || end_scalar > domain.raw_scalar_len {
        return Err(StoryEditDomainError::new(
            "invalid_range",
            "ordinary Story scalar range is invalid",
        ));
    }
    let editable_start = domain.editable_start_scalar.ok_or_else(|| {
        StoryEditDomainError::new("edit_domain_unknown", "editable start is unavailable")
    })?;
    let editable_end = domain.editable_end_scalar.ok_or_else(|| {
        StoryEditDomainError::new("edit_domain_unknown", "editable end is unavailable")
    })?;
    if start_scalar < editable_start {
        return Err(StoryEditDomainError::new(
            "invalid_range",
            "ordinary Story range starts before editable content",
        ));
    }
    if start_scalar == end_scalar && start_scalar == editable_end {
        return Ok(());
    }
    if end_scalar > editable_end || start_scalar > editable_end {
        if !domain.protected_ranges.is_empty() {
            return Err(StoryEditDomainError::new(
                "protected_story_structure",
                "ordinary Story range overlaps protected source structure",
            ));
        }
        return Err(StoryEditDomainError::new(
            "invalid_range",
            "ordinary Story range exceeds editable content",
        ));
    }
    Ok(())
}

pub fn edit_domain_id_v1(domain: &StoryEditDomainV1) -> String {
    let protected = domain
        .protected_ranges
        .iter()
        .map(|item| {
            json!({
                "start_scalar": item.start_scalar,
                "end_scalar": item.end_scalar,
                "reason": item.reason,
                "provenance": item.provenance.as_str(),
            })
        })
        .collect::<Vec<_>>();
    let value = json!({
        "protocol_version": domain.protocol_version,
        "story_id": domain.story_id,
        "provenance": domain.provenance.as_str(),
        "status": domain.status,
        "raw_scalar_len": domain.raw_scalar_len,
        "editable_start_scalar": domain.editable_start_scalar,
        "editable_end_scalar": domain.editable_end_scalar,
        "caret_start_boundary": domain.caret_start_boundary,
        "caret_end_boundary": domain.caret_end_boundary,
        "protected_ranges": protected,
    });
    let bytes = serde_json::to_vec(&value).expect("StoryEditDomainV1 JSON");
    let mut digest = Sha256::new();
    digest.update(bytes);
    format!("sha256:{:x}", digest.finalize())
}

pub fn to_interaction_domain_v1(
    domain: &StoryEditDomainV1,
) -> chaptera_text_interaction_adapter::StoryEditDomainV1 {
    chaptera_text_interaction_adapter::StoryEditDomainV1 {
        story_id: domain.story_id.clone(),
        raw_scalar_len: domain.raw_scalar_len,
        status: domain.status.clone(),
        caret_start_boundary: domain.caret_start_boundary,
        caret_end_boundary: domain.caret_end_boundary,
        domain_id: edit_domain_id_v1(domain),
    }
}

fn is_exact_primary_story_ref(
    session: &EditorSession,
    reference: &pub_model::SourceRef,
    role: SourceRole,
    path: &str,
) -> bool {
    reference.validate_primary_source(&session.graph().source).is_ok()
        && reference.carrier == pub_reader::QUILL_STREAM_PATH
        && reference.path.as_deref() == Some(path)
        && reference.role == role
        && reference.authority == AuthorityClass::Authoritative
        && reference.confidence == Some(ReadConfidence::Exact)
        && reference
            .object_key
            .as_deref()
            .is_some_and(|key| key.starts_with("quill/syid/"))
}

pub fn derive_editor_story_provenance_v1(
    session: &EditorSession,
    story_id: StoryId,
    hint: StoryProvenanceHintV1,
) -> Result<StoryProvenanceV1, StoryEditDomainError> {
    let story = session.graph().stories.get(&story_id).ok_or_else(|| {
        StoryEditDomainError::new("missing_story", "Story is absent from current EditorSession")
    })?;

    if hint == StoryProvenanceHintV1::ChapteraCreated {
        let carries_primary_source = story
            .source_refs
            .iter()
            .any(|reference| reference.validate_primary_source(&session.graph().source).is_ok());
        if carries_primary_source {
            return Err(StoryEditDomainError::new(
                "provenance_conflict",
                "Chaptera-created provenance conflicts with persisted primary-source Story refs",
            ));
        }
        return Ok(StoryProvenanceV1::ChapteraCreated);
    }

    let source = &session.graph().source;
    let profile_is_mature_0x2c = source.format == "pub"
        && source.format_version.as_deref() == Some("0x2c")
        && source.adapter_version.starts_with("pub-rs/");

    let exact_syid_keys = story
        .source_refs
        .iter()
        .filter(|reference| {
            is_exact_primary_story_ref(session, reference, SourceRole::Relation, "SYID")
        })
        .filter_map(|reference| reference.object_key.clone())
        .collect::<BTreeSet<_>>();
    let exact_text_keys = story
        .source_refs
        .iter()
        .filter(|reference| {
            is_exact_primary_story_ref(session, reference, SourceRole::Semantic, "TEXT")
        })
        .filter_map(|reference| reference.object_key.clone())
        .collect::<BTreeSet<_>>();
    let confirmed_persisted_quill_story = exact_syid_keys
        .iter()
        .any(|key| exact_text_keys.contains(key));

    if profile_is_mature_0x2c
        && confirmed_persisted_quill_story
        && story.text.ends_with('\r')
    {
        Ok(StoryProvenanceV1::ImportedMatureQuillTerminalCr)
    } else {
        Ok(StoryProvenanceV1::ImportedUnknown)
    }
}

pub fn derive_editor_story_edit_domain_v1(
    session: &EditorSession,
    story_id: StoryId,
    hint: StoryProvenanceHintV1,
) -> Result<StoryEditDomainV1, StoryEditDomainError> {
    let story = session.graph().stories.get(&story_id).ok_or_else(|| {
        StoryEditDomainError::new("missing_story", "Story is absent from current EditorSession")
    })?;
    let provenance = derive_editor_story_provenance_v1(session, story_id, hint)?;
    derive_story_edit_domain_v1(
        story_id.as_canonical().to_string(),
        &story.text,
        provenance,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chaptera_final_cr_is_not_auto_protected() {
        let domain = derive_story_edit_domain_v1(
            "story:a",
            "A\r",
            StoryProvenanceV1::ChapteraCreated,
        )
        .unwrap();
        assert_eq!(domain.editable_end_scalar, Some(2));
        assert!(domain.protected_ranges.is_empty());
    }

    #[test]
    fn mature_terminal_cr_protects_only_final_scalar() {
        let domain = derive_story_edit_domain_v1(
            "story:q",
            "A\rB\r",
            StoryProvenanceV1::ImportedMatureQuillTerminalCr,
        )
        .unwrap();
        assert_eq!(domain.editable_end_scalar, Some(3));
        assert_eq!(domain.protected_ranges[0].start_scalar, 3);
        validate_ordinary_story_range_v1(&domain, 1, 2).unwrap();
        assert_eq!(
            validate_ordinary_story_range_v1(&domain, 2, 4)
                .unwrap_err()
                .code,
            "protected_story_structure"
        );
    }

    #[test]
    fn unknown_import_fails_closed_even_with_trailing_cr() {
        let domain = derive_story_edit_domain_v1(
            "story:u",
            "ABC\r",
            StoryProvenanceV1::ImportedUnknown,
        )
        .unwrap();
        assert_eq!(domain.status, "edit_domain_unknown");
        assert_eq!(
            validate_ordinary_story_range_v1(&domain, 0, 3)
                .unwrap_err()
                .code,
            "edit_domain_unknown"
        );
    }
}
