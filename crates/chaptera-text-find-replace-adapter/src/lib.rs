use chaptera_text_input_adapter::domain::{edit_domain_id_v1, StoryEditDomainV1};
use chaptera_text_input_adapter::ingress::normalize_external_text_v1;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::fmt;

pub const TEXT_FIND_SNAPSHOT_VERSION_V1: &str = "chaptera.text-find-snapshot.v1";
pub const TEXT_FIND_POLICY_VERSION_V1: &str = "chaptera.text-find-policy.v1";
pub const STORY_FIND_REPLACE_PLAN_VERSION_V1: &str = "chaptera.story-find-replace-plan.v1";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextFindReplaceError {
    pub code: &'static str,
    pub message: String,
}

impl TextFindReplaceError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl fmt::Display for TextFindReplaceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for TextFindReplaceError {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TextFindExtentV1 {
    FullEditableStory,
    Range {
        start_scalar: u32,
        end_scalar: u32,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TextFindMatchV1 {
    pub ordinal: u32,
    pub start_scalar: u32,
    pub end_scalar: u32,
    pub matched_text: String,
    pub matched_text_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TextFindSnapshotV1 {
    pub protocol_version: String,
    pub policy_version: String,
    pub revision_id: String,
    pub story_id: String,
    pub query: String,
    pub extent_start_scalar: u32,
    pub extent_end_scalar: u32,
    pub matches: Vec<TextFindMatchV1>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextFindDirectionV1 {
    Next,
    Previous,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TextProgrammaticJumpIntentV1 {
    pub protocol_version: String,
    pub document_id: String,
    pub revision_id: String,
    pub story_id: String,
    pub start_scalar: u32,
    pub end_scalar: u32,
    pub selection_mode: String,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoryFindReplaceEditV1 {
    pub match_ordinal: u32,
    pub start_scalar: u32,
    pub end_scalar: u32,
    pub expected_before: String,
    pub replacement_text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoryFindReplacePlanV1 {
    pub protocol_version: String,
    pub base_revision_id: String,
    pub story_id: String,
    pub edit_domain_id: String,
    pub story_text_sha256: String,
    pub replacement_text: String,
    pub edits: Vec<StoryFindReplaceEditV1>,
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut digest = Sha256::new();
    digest.update(bytes);
    format!("{:x}", digest.finalize())
}

fn scalar_len(text: &str) -> Result<u32, TextFindReplaceError> {
    u32::try_from(text.chars().count())
        .map_err(|_| TextFindReplaceError::new("scalar_overflow", "Story scalar length exceeds u32"))
}

fn scalar_slice(text: &str, start_scalar: u32, end_scalar: u32) -> Option<&str> {
    if end_scalar < start_scalar {
        return None;
    }
    let start_index = usize::try_from(start_scalar).ok()?;
    let end_index = usize::try_from(end_scalar).ok()?;
    let scalar_count = text.chars().count();
    if end_index > scalar_count {
        return None;
    }
    let start_byte = if start_index == scalar_count {
        text.len()
    } else {
        text.char_indices().nth(start_index)?.0
    };
    let end_byte = if end_index == scalar_count {
        text.len()
    } else {
        text.char_indices().nth(end_index)?.0
    };
    text.get(start_byte..end_byte)
}

fn editable_bounds(
    domain: &StoryEditDomainV1,
) -> Result<(u32, u32), TextFindReplaceError> {
    if domain.status != "known" {
        return Err(TextFindReplaceError::new(
            "edit_domain_unknown",
            "Find requires one known ordinary editable Story domain",
        ));
    }
    let start = domain.editable_start_scalar.ok_or_else(|| {
        TextFindReplaceError::new("edit_domain_unknown", "editable start is unavailable")
    })?;
    let end = domain.editable_end_scalar.ok_or_else(|| {
        TextFindReplaceError::new("edit_domain_unknown", "editable end is unavailable")
    })?;
    if end < start || end > domain.raw_scalar_len {
        return Err(TextFindReplaceError::new(
            "invalid_edit_domain",
            "editable Story bounds are inconsistent",
        ));
    }
    Ok((start, end))
}

fn resolve_extent(
    domain: &StoryEditDomainV1,
    extent: &TextFindExtentV1,
) -> Result<(u32, u32), TextFindReplaceError> {
    let (editable_start, editable_end) = editable_bounds(domain)?;
    match extent {
        TextFindExtentV1::FullEditableStory => Ok((editable_start, editable_end)),
        TextFindExtentV1::Range {
            start_scalar,
            end_scalar,
        } => {
            if start_scalar >= end_scalar
                || *start_scalar < editable_start
                || *end_scalar > editable_end
            {
                return Err(TextFindReplaceError::new(
                    "invalid_extent",
                    "explicit Find range must be non-empty and wholly inside editable Story content",
                ));
            }
            Ok((*start_scalar, *end_scalar))
        }
    }
}

fn validate_story_against_domain(
    story_text: &str,
    domain: &StoryEditDomainV1,
) -> Result<(), TextFindReplaceError> {
    let len = scalar_len(story_text)?;
    if len != domain.raw_scalar_len {
        return Err(TextFindReplaceError::new(
            "find_snapshot_stale",
            "StoryEditDomain no longer matches Story length",
        ));
    }
    Ok(())
}

pub fn build_text_find_snapshot_v1(
    base_revision_id: &str,
    story_text: &str,
    domain: &StoryEditDomainV1,
    extent: TextFindExtentV1,
    external_query: &str,
) -> Result<TextFindSnapshotV1, TextFindReplaceError> {
    if base_revision_id.is_empty() {
        return Err(TextFindReplaceError::new(
            "invalid_revision",
            "base RevisionId is required",
        ));
    }
    validate_story_against_domain(story_text, domain)?;
    let (extent_start, extent_end) = resolve_extent(domain, &extent)?;

    let query = normalize_external_text_v1(external_query)
        .map_err(|error| TextFindReplaceError::new("invalid_query", error.to_string()))?
        .text;
    if query.is_empty() {
        return Err(TextFindReplaceError::new(
            "empty_query",
            "Find query must be non-empty",
        ));
    }

    let story_scalars: Vec<char> = story_text.chars().collect();
    let query_scalars: Vec<char> = query.chars().collect();
    let query_len = u32::try_from(query_scalars.len())
        .map_err(|_| TextFindReplaceError::new("scalar_overflow", "query is too long"))?;

    let mut matches = Vec::new();
    let mut cursor = extent_start;
    while cursor < extent_end {
        let candidate_end = cursor
            .checked_add(query_len)
            .ok_or_else(|| TextFindReplaceError::new("scalar_overflow", "match boundary overflow"))?;
        if candidate_end > extent_end {
            break;
        }
        let start = usize::try_from(cursor)
            .map_err(|_| TextFindReplaceError::new("scalar_overflow", "match start overflow"))?;
        let end = usize::try_from(candidate_end)
            .map_err(|_| TextFindReplaceError::new("scalar_overflow", "match end overflow"))?;
        if story_scalars[start..end] == query_scalars[..] {
            let matched_text = scalar_slice(story_text, cursor, candidate_end)
                .ok_or_else(|| {
                    TextFindReplaceError::new(
                        "invalid_story",
                        "matched scalar range could not be materialized",
                    )
                })?
                .to_owned();
            let ordinal = u32::try_from(matches.len())
                .map_err(|_| TextFindReplaceError::new("match_count_overflow", "too many matches"))?;
            matches.push(TextFindMatchV1 {
                ordinal,
                start_scalar: cursor,
                end_scalar: candidate_end,
                matched_text_sha256: sha256_hex(matched_text.as_bytes()),
                matched_text,
            });
            cursor = candidate_end;
        } else {
            cursor = cursor
                .checked_add(1)
                .ok_or_else(|| TextFindReplaceError::new("scalar_overflow", "scan overflow"))?;
        }
    }

    Ok(TextFindSnapshotV1 {
        protocol_version: TEXT_FIND_SNAPSHOT_VERSION_V1.to_owned(),
        policy_version: TEXT_FIND_POLICY_VERSION_V1.to_owned(),
        revision_id: base_revision_id.to_owned(),
        story_id: domain.story_id.clone(),
        query,
        extent_start_scalar: extent_start,
        extent_end_scalar: extent_end,
        matches,
    })
}

pub fn validate_text_find_snapshot_v1(
    snapshot: &TextFindSnapshotV1,
    current_revision_id: &str,
    current_story_text: &str,
    current_domain: &StoryEditDomainV1,
) -> Result<(), TextFindReplaceError> {
    if snapshot.protocol_version != TEXT_FIND_SNAPSHOT_VERSION_V1
        || snapshot.policy_version != TEXT_FIND_POLICY_VERSION_V1
        || snapshot.revision_id != current_revision_id
        || snapshot.story_id != current_domain.story_id
    {
        return Err(TextFindReplaceError::new(
            "find_snapshot_stale",
            "snapshot revision/story identity changed",
        ));
    }
    validate_story_against_domain(current_story_text, current_domain)?;
    let (editable_start, editable_end) = editable_bounds(current_domain)?;
    if snapshot.extent_start_scalar < editable_start
        || snapshot.extent_end_scalar > editable_end
    {
        return Err(TextFindReplaceError::new(
            "find_snapshot_stale",
            "snapshot extent is no longer editable",
        ));
    }
    for item in &snapshot.matches {
        if scalar_slice(current_story_text, item.start_scalar, item.end_scalar)
            != Some(item.matched_text.as_str())
        {
            return Err(TextFindReplaceError::new(
                "find_snapshot_stale",
                "snapshot match no longer equals canonical Story",
            ));
        }
    }
    Ok(())
}

pub fn navigate_text_find_snapshot_v1(
    snapshot: &TextFindSnapshotV1,
    direction: TextFindDirectionV1,
    navigation_origin_scalar: u32,
    wrap: bool,
) -> Option<&TextFindMatchV1> {
    if snapshot.matches.is_empty() {
        return None;
    }
    match direction {
        TextFindDirectionV1::Next => snapshot
            .matches
            .iter()
            .find(|item| item.start_scalar >= navigation_origin_scalar)
            .or_else(|| wrap.then(|| snapshot.matches.first()).flatten()),
        TextFindDirectionV1::Previous => snapshot
            .matches
            .iter()
            .rev()
            .find(|item| item.end_scalar <= navigation_origin_scalar)
            .or_else(|| wrap.then(|| snapshot.matches.last()).flatten()),
    }
}

pub fn programmatic_jump_intent_for_match_v1(
    document_id: &str,
    snapshot: &TextFindSnapshotV1,
    ordinal: u32,
) -> Result<TextProgrammaticJumpIntentV1, TextFindReplaceError> {
    let item = snapshot
        .matches
        .get(usize::try_from(ordinal).map_err(|_| {
            TextFindReplaceError::new("invalid_match_ordinal", "match ordinal overflows usize")
        })?)
        .filter(|item| item.ordinal == ordinal)
        .ok_or_else(|| {
            TextFindReplaceError::new(
                "invalid_match_ordinal",
                "match ordinal is absent from this immutable snapshot",
            )
        })?;
    if document_id.is_empty() {
        return Err(TextFindReplaceError::new(
            "invalid_jump",
            "document_id is required",
        ));
    }
    Ok(TextProgrammaticJumpIntentV1 {
        protocol_version: "chaptera.text-programmatic-jump.v1".to_owned(),
        document_id: document_id.to_owned(),
        revision_id: snapshot.revision_id.clone(),
        story_id: snapshot.story_id.clone(),
        start_scalar: item.start_scalar,
        end_scalar: item.end_scalar,
        selection_mode: "exact_range".to_owned(),
        reason: "find_result".to_owned(),
    })
}

pub fn build_story_find_replace_plan_v1(
    snapshot: &TextFindSnapshotV1,
    selected_ordinals: &[u32],
    current_revision_id: &str,
    current_story_text: &str,
    current_domain: &StoryEditDomainV1,
    external_replacement_text: &str,
) -> Result<StoryFindReplacePlanV1, TextFindReplaceError> {
    validate_text_find_snapshot_v1(
        snapshot,
        current_revision_id,
        current_story_text,
        current_domain,
    )?;
    if selected_ordinals.is_empty() {
        return Err(TextFindReplaceError::new(
            "empty_match_selection",
            "Replace requires at least one frozen snapshot match",
        ));
    }

    let replacement = normalize_external_text_v1(external_replacement_text)
        .map_err(|error| TextFindReplaceError::new("invalid_replacement", error.to_string()))?
        .text;

    let mut seen = BTreeSet::new();
    let mut edits = Vec::with_capacity(selected_ordinals.len());
    for &ordinal in selected_ordinals {
        if !seen.insert(ordinal) {
            return Err(TextFindReplaceError::new(
                "duplicate_match_ordinal",
                "Replace match ordinal list must not contain duplicates",
            ));
        }
        let item = snapshot
            .matches
            .get(usize::try_from(ordinal).map_err(|_| {
                TextFindReplaceError::new("invalid_match_ordinal", "match ordinal overflows usize")
            })?)
            .filter(|item| item.ordinal == ordinal)
            .ok_or_else(|| {
                TextFindReplaceError::new(
                    "invalid_match_ordinal",
                    "selected ordinal is absent from the immutable Find snapshot",
                )
            })?;
        let current = scalar_slice(current_story_text, item.start_scalar, item.end_scalar)
            .ok_or_else(|| {
                TextFindReplaceError::new(
                    "find_snapshot_stale",
                    "selected match range is absent from current Story text",
                )
            })?;
        if current != item.matched_text || sha256_hex(current.as_bytes()) != item.matched_text_sha256
        {
            return Err(TextFindReplaceError::new(
                "find_snapshot_stale",
                "selected match text no longer equals the frozen snapshot",
            ));
        }
        edits.push(StoryFindReplaceEditV1 {
            match_ordinal: ordinal,
            start_scalar: item.start_scalar,
            end_scalar: item.end_scalar,
            expected_before: item.matched_text.clone(),
            replacement_text: replacement.clone(),
        });
    }
    edits.sort_by_key(|item| (item.start_scalar, item.end_scalar, item.match_ordinal));
    for pair in edits.windows(2) {
        if pair[0].end_scalar > pair[1].start_scalar {
            return Err(TextFindReplaceError::new(
                "overlapping_matches",
                "selected frozen Find matches overlap",
            ));
        }
    }

    Ok(StoryFindReplacePlanV1 {
        protocol_version: STORY_FIND_REPLACE_PLAN_VERSION_V1.to_owned(),
        base_revision_id: snapshot.revision_id.clone(),
        story_id: snapshot.story_id.clone(),
        edit_domain_id: edit_domain_id_v1(current_domain),
        story_text_sha256: sha256_hex(current_story_text.as_bytes()),
        replacement_text: replacement,
        edits,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chaptera_text_input_adapter::domain::{
        derive_story_edit_domain_v1, StoryProvenanceV1,
    };

    fn domain(text: &str) -> StoryEditDomainV1 {
        derive_story_edit_domain_v1("story:1", text, StoryProvenanceV1::ChapteraCreated).unwrap()
    }

    #[test]
    fn exact_search_is_scalar_based_and_non_overlapping() {
        let text = "aaa😀aa";
        let snapshot = build_text_find_snapshot_v1(
            "rev:1",
            text,
            &domain(text),
            TextFindExtentV1::FullEditableStory,
            "aa",
        )
        .unwrap();
        assert_eq!(
            snapshot
                .matches
                .iter()
                .map(|item| (item.start_scalar, item.end_scalar))
                .collect::<Vec<_>>(),
            vec![(0, 2), (4, 6)]
        );
    }

    #[test]
    fn query_newline_spelling_normalizes_but_unicode_does_not() {
        let text = "A\rB\re\u{301}\ré";
        let d = domain(text);
        let newline = build_text_find_snapshot_v1(
            "rev:1",
            text,
            &d,
            TextFindExtentV1::FullEditableStory,
            "\n",
        )
        .unwrap();
        assert_eq!(newline.matches.len(), 3);

        let combining = build_text_find_snapshot_v1(
            "rev:1",
            text,
            &d,
            TextFindExtentV1::FullEditableStory,
            "e\u{301}",
        )
        .unwrap();
        assert_eq!(combining.matches.len(), 1);
        let precomposed = build_text_find_snapshot_v1(
            "rev:1",
            text,
            &d,
            TextFindExtentV1::FullEditableStory,
            "é",
        )
        .unwrap();
        assert_eq!(precomposed.matches.len(), 1);
    }

    #[test]
    fn protected_terminal_cr_is_excluded_from_full_editable_story() {
        let text = "A\r";
        let d = derive_story_edit_domain_v1(
            "story:1",
            text,
            StoryProvenanceV1::ImportedMatureQuillTerminalCr,
        )
        .unwrap();
        let snapshot = build_text_find_snapshot_v1(
            "rev:1",
            text,
            &d,
            TextFindExtentV1::FullEditableStory,
            "\r",
        )
        .unwrap();
        assert!(snapshot.matches.is_empty());
    }

    #[test]
    fn explicit_extent_never_clips_crossing_candidate() {
        let text = "XabcYabcZ";
        let snapshot = build_text_find_snapshot_v1(
            "rev:1",
            text,
            &domain(text),
            TextFindExtentV1::Range {
                start_scalar: 2,
                end_scalar: 8,
            },
            "abc",
        )
        .unwrap();
        assert_eq!(
            snapshot
                .matches
                .iter()
                .map(|item| (item.start_scalar, item.end_scalar))
                .collect::<Vec<_>>(),
            vec![(5, 8)]
        );
    }

    #[test]
    fn regenerated_snapshot_navigation_uses_canonical_origin_not_old_ordinal() {
        let before = "aa aa";
        let d1 = domain(before);
        let snapshot = build_text_find_snapshot_v1(
            "rev:1",
            before,
            &d1,
            TextFindExtentV1::FullEditableStory,
            "aa",
        )
        .unwrap();
        assert_eq!(
            navigate_text_find_snapshot_v1(&snapshot, TextFindDirectionV1::Next, 0, false)
                .unwrap()
                .ordinal,
            0
        );

        let after = "X aa aa";
        let d2 = domain(after);
        let refreshed = build_text_find_snapshot_v1(
            "rev:2",
            after,
            &d2,
            TextFindExtentV1::FullEditableStory,
            "aa",
        )
        .unwrap();
        let next =
            navigate_text_find_snapshot_v1(&refreshed, TextFindDirectionV1::Next, 4, false).unwrap();
        assert_eq!((next.start_scalar, next.end_scalar), (5, 7));
    }

    #[test]
    fn stale_snapshot_is_rejected_after_revision_or_text_change() {
        let text = "abc abc";
        let d = domain(text);
        let snapshot = build_text_find_snapshot_v1(
            "rev:1",
            text,
            &d,
            TextFindExtentV1::FullEditableStory,
            "abc",
        )
        .unwrap();
        assert_eq!(
            validate_text_find_snapshot_v1(&snapshot, "rev:2", text, &d)
                .unwrap_err()
                .code,
            "find_snapshot_stale"
        );
        let changed = "xbc abc";
        let d2 = domain(changed);
        assert_eq!(
            validate_text_find_snapshot_v1(&snapshot, "rev:1", changed, &d2)
                .unwrap_err()
                .code,
            "find_snapshot_stale"
        );
    }

    #[test]
    fn replace_plan_is_frozen_base_coordinate_and_empty_replacement_is_delete() {
        let text = "aa--aa";
        let d = domain(text);
        let snapshot = build_text_find_snapshot_v1(
            "rev:1",
            text,
            &d,
            TextFindExtentV1::FullEditableStory,
            "aa",
        )
        .unwrap();
        let plan = build_story_find_replace_plan_v1(
            &snapshot,
            &[1, 0],
            "rev:1",
            text,
            &d,
            "",
        )
        .unwrap();
        assert_eq!(
            plan.edits
                .iter()
                .map(|edit| (edit.match_ordinal, edit.start_scalar, edit.end_scalar))
                .collect::<Vec<_>>(),
            vec![(0, 0, 2), (1, 4, 6)]
        );
        assert!(plan.edits.iter().all(|edit| edit.replacement_text.is_empty()));
    }

    #[test]
    fn zero_match_or_duplicate_selection_does_not_create_replace_plan() {
        let text = "abc";
        let d = domain(text);
        let snapshot = build_text_find_snapshot_v1(
            "rev:1",
            text,
            &d,
            TextFindExtentV1::FullEditableStory,
            "z",
        )
        .unwrap();
        assert_eq!(
            build_story_find_replace_plan_v1(&snapshot, &[], "rev:1", text, &d, "x")
                .unwrap_err()
                .code,
            "empty_match_selection"
        );

        let snapshot2 = build_text_find_snapshot_v1(
            "rev:1",
            text,
            &d,
            TextFindExtentV1::FullEditableStory,
            "a",
        )
        .unwrap();
        assert_eq!(
            build_story_find_replace_plan_v1(&snapshot2, &[0, 0], "rev:1", text, &d, "x")
                .unwrap_err()
                .code,
            "duplicate_match_ordinal"
        );
    }

    #[test]
    fn jump_intent_is_exact_semantic_range_only() {
        let text = "abc abc";
        let snapshot = build_text_find_snapshot_v1(
            "rev:1",
            text,
            &domain(text),
            TextFindExtentV1::FullEditableStory,
            "abc",
        )
        .unwrap();
        let jump = programmatic_jump_intent_for_match_v1("doc:1", &snapshot, 1).unwrap();
        assert_eq!((jump.start_scalar, jump.end_scalar), (4, 7));
        assert_eq!(jump.story_id, "story:1");
        assert_eq!(jump.protocol_version, "chaptera.text-programmatic-jump.v1");
        assert_eq!(jump.selection_mode, "exact_range");
    }
}
