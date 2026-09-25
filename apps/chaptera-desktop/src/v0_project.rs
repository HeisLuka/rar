use pub_editor::{EditOperation, EditorSession, LengthEmu, NodeId, RectEmu, StoryId};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const V0_PROJECT_SCHEMA_VERSION: &str = "pub-editor-v0.4";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DesktopV0Project {
    pub schema_version: String,
    pub source_hash: String,
    pub operations: Vec<DesktopV0Operation>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DesktopV0Operation {
    ReplaceStoryRange {
        story_id: String,
        scalar_start: u32,
        scalar_end: u32,
        replacement_text: String,
        before_story_state_id: String,
        after_story_state_id: String,
    },
    MoveNode {
        node_id: String,
        before: RectWire,
        after: RectWire,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RectWire {
    pub x: i64,
    pub y: i64,
    pub width: i64,
    pub height: i64,
}

impl RectWire {
    fn from_rect(value: RectEmu) -> Self {
        Self {
            x: value.x.get(),
            y: value.y.get(),
            width: value.width.get(),
            height: value.height.get(),
        }
    }

    fn to_rect(self) -> Result<RectEmu, String> {
        if self.width <= 0 || self.height <= 0 {
            return Err("desktop V0 MoveNode requires positive width/height".to_owned());
        }
        Ok(RectEmu::new(
            LengthEmu::new(self.x),
            LengthEmu::new(self.y),
            LengthEmu::new(self.width),
            LengthEmu::new(self.height),
        ))
    }
}

pub fn validate_full_story_replacement(before: &str, after: &str) -> Result<(), String> {
    if before == after {
        return Err("replacement Story text is unchanged".to_owned());
    }
    if before.ends_with('\r') && !after.ends_with('\r') {
        return Err(
            "replacement would remove the source-backed terminal CR required by Desktop V0"
                .to_owned(),
        );
    }
    Ok(())
}

/// Exact Rust mirror of services/editor-api/story_range_v1.py::story_state_id_v1.
///
/// Key order is deliberately fixed to the canonical JSON order used by the
/// Python authority: protocol_version, story_id, text. serde_json string
/// encoding keeps Unicode as UTF-8 rather than ASCII escapes, matching
/// ensure_ascii=False.
pub fn story_state_id_v1(story_id: &str, text: &str) -> String {
    let protocol = serde_json::to_string("chaptera.story-state.v1")
        .expect("constant Story state protocol must serialize");
    let story_id = serde_json::to_string(story_id).expect("StoryId text must serialize");
    let text = serde_json::to_string(text).expect("Story text must serialize");
    let payload = format!(
        "{{\"protocol_version\":{protocol},\"story_id\":{story_id},\"text\":{text}}}"
    );
    format!("sha256:{:x}", Sha256::digest(payload.as_bytes()))
}

pub fn encode_project_v0(editor: &EditorSession) -> Result<Vec<u8>, String> {
    let operations = editor
        .operations()
        .iter()
        .map(translate_operation)
        .collect::<Result<Vec<_>, _>>()?;
    if operations.is_empty() {
        return Err("there are no Desktop V0 edit operations to save".to_owned());
    }
    let project = DesktopV0Project {
        schema_version: V0_PROJECT_SCHEMA_VERSION.to_owned(),
        source_hash: editor.source_hash().to_string(),
        operations,
    };
    serde_json::to_vec_pretty(&project).map_err(|error| format!("serialize Desktop V0 project: {error}"))
}

pub fn decode_project_v0(bytes: &[u8]) -> Result<DesktopV0Project, String> {
    let project: DesktopV0Project =
        serde_json::from_slice(bytes).map_err(|error| format!("parse Desktop V0 project: {error}"))?;
    if project.schema_version != V0_PROJECT_SCHEMA_VERSION {
        return Err(format!(
            "unsupported Desktop V0 project schema {}",
            project.schema_version
        ));
    }
    Ok(project)
}

pub fn apply_project_v0(
    editor: &mut EditorSession,
    project: &DesktopV0Project,
) -> Result<usize, String> {
    if project.schema_version != V0_PROJECT_SCHEMA_VERSION {
        return Err("Desktop V0 project schema mismatch".to_owned());
    }
    if project.source_hash != editor.source_hash().to_string() {
        return Err("Desktop V0 project source identity mismatch".to_owned());
    }

    for operation in &project.operations {
        match operation {
            DesktopV0Operation::ReplaceStoryRange {
                story_id,
                scalar_start,
                scalar_end,
                replacement_text,
                before_story_state_id,
                after_story_state_id,
            } => {
                let story_id_typed = parse_story_id(story_id)?;
                editor
                    .can_replace_story_text(story_id_typed)
                    .map_err(|error| format!("Story capability rejected during replay: {error}"))?;
                let before = editor
                    .graph()
                    .stories
                    .get(&story_id_typed)
                    .ok_or_else(|| "Desktop V0 Story is absent during replay".to_owned())?
                    .text
                    .clone();

                let before_len = u32::try_from(before.chars().count())
                    .map_err(|_| "Desktop V0 Story scalar length exceeds u32".to_owned())?;
                if *scalar_start != 0 || *scalar_end != before_len {
                    return Err(
                        "Desktop V0 currently admits only canonical whole-Story range replacement"
                            .to_owned(),
                    );
                }
                validate_full_story_replacement(&before, replacement_text)?;
                if story_state_id_v1(story_id, &before) != *before_story_state_id {
                    return Err("Desktop V0 Story before-state identity mismatch".to_owned());
                }
                if story_state_id_v1(story_id, replacement_text) != *after_story_state_id {
                    return Err("Desktop V0 Story after-state identity mismatch".to_owned());
                }
                editor
                    .replace_story_text(story_id_typed, replacement_text.clone())
                    .map_err(|error| format!("replay Story range replacement: {error}"))?;
            }
            DesktopV0Operation::MoveNode {
                node_id,
                before,
                after,
            } => {
                let node_id_typed = parse_node_id(node_id)?;
                let before_rect = before.to_rect()?;
                let after_rect = after.to_rect()?;
                let authored = editor
                    .graph()
                    .nodes
                    .get(&node_id_typed)
                    .ok_or_else(|| "Desktop V0 MoveNode target is absent during replay".to_owned())?;
                if authored.header.bounds != before_rect {
                    return Err("Desktop V0 MoveNode before geometry is stale".to_owned());
                }
                if before_rect.width != after_rect.width || before_rect.height != after_rect.height {
                    return Err("Desktop V0 MoveNode cannot resize during replay".to_owned());
                }
                editor
                    .move_node_to(node_id_typed, after_rect.x, after_rect.y)
                    .map_err(|error| format!("replay Desktop V0 MoveNode: {error}"))?;
                let replayed = editor
                    .graph()
                    .nodes
                    .get(&node_id_typed)
                    .ok_or_else(|| "Desktop V0 MoveNode target disappeared".to_owned())?
                    .header
                    .bounds;
                if replayed != after_rect {
                    return Err("Desktop V0 MoveNode replay geometry mismatch".to_owned());
                }
            }
        }
    }

    Ok(project.operations.len())
}

fn translate_operation(operation: &EditOperation) -> Result<DesktopV0Operation, String> {
    match operation {
        EditOperation::ReplaceStoryText {
            story_id,
            before,
            after,
        } => {
            validate_full_story_replacement(before, after)?;
            let story_id = story_id.as_canonical().to_string();
            let scalar_end = u32::try_from(before.chars().count())
                .map_err(|_| "Desktop V0 Story scalar length exceeds u32".to_owned())?;
            Ok(DesktopV0Operation::ReplaceStoryRange {
                before_story_state_id: story_state_id_v1(&story_id, before),
                after_story_state_id: story_state_id_v1(&story_id, after),
                story_id,
                scalar_start: 0,
                scalar_end,
                replacement_text: after.clone(),
            })
        }
        EditOperation::MoveNode {
            node_id,
            before,
            after,
        } => Ok(DesktopV0Operation::MoveNode {
            node_id: node_id.as_canonical().to_string(),
            before: RectWire::from_rect(*before),
            after: RectWire::from_rect(*after),
        }),
        EditOperation::ReplaceTableCellText { .. } => Err(
            "Desktop Portable V0 sidecar does not yet admit table-cell operations".to_owned(),
        ),
        EditOperation::ReplaceImage { .. } => {
            Err("Desktop Portable V0 sidecar does not yet admit ReplaceImage".to_owned())
        }
    }
}

fn parse_story_id(value: &str) -> Result<StoryId, String> {
    serde_json::from_value(serde_json::Value::String(value.to_owned()))
        .map_err(|error| format!("invalid Desktop V0 StoryId: {error}"))
}

fn parse_node_id(value: &str) -> Result<NodeId, String> {
    serde_json::from_value(serde_json::Value::String(value.to_owned()))
        .map_err(|error| format!("invalid Desktop V0 NodeId: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn story_state_id_matches_python_authority_ascii_vector() {
        assert_eq!(
            story_state_id_v1("00112233-4455-6677-8899-aabbccddeeff", "abc"),
            "sha256:e04a23e328e62ef413ba67e959a6def0a764855255348e6b8761b781f4ad8b48"
        );
    }

    #[test]
    fn story_state_id_is_scalar_text_sensitive() {
        let story = "00112233-4455-6677-8899-aabbccddeeff";
        assert_ne!(story_state_id_v1(story, "a"), story_state_id_v1(story, "á"));
    }

    #[test]
    fn terminal_cr_is_fail_closed() {
        assert!(validate_full_story_replacement("before\r", "after").is_err());
        assert!(validate_full_story_replacement("before\r", "after\r").is_ok());
    }
}
