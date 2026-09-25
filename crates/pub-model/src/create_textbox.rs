use serde::{Deserialize, Serialize};

use crate::{
    AuthoringTextPresetError, AuthoringTextPresetV1, RectEmuV1, authoring_text_preset_id_v1,
    validate_rect_emu_v1, validate_uuid_v7_v1,
};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CreateTextBoxV1 {
    pub node_id: String,
    pub story_id: String,
    pub page_id: String,
    pub bounds: RectEmuV1,
    pub text_preset: AuthoringTextPresetV1,
    pub initial_text: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TextBoxProvenanceV1 {
    AuthorCreated,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AuthoredTextFrameV1 {
    pub node_id: String,
    pub page_id: String,
    pub parent_id: String,
    pub story_id: String,
    pub bounds: RectEmuV1,
    pub text_preset_id: String,
    pub provenance: TextBoxProvenanceV1,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AuthoringStorySeedV1 {
    pub story_id: String,
    pub text_preset_id: String,
    pub text_preset: AuthoringTextPresetV1,
    pub initial_text: Option<String>,
    pub provenance: TextBoxProvenanceV1,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CreateTextBoxPlanV1 {
    pub frame: AuthoredTextFrameV1,
    pub story: AuthoringStorySeedV1,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CreateTextBoxError {
    InvalidNodeId,
    InvalidStoryId,
    EmptyPageId,
    InvalidBounds,
    InvalidPreset(AuthoringTextPresetError),
}

pub fn create_textbox_plan_v1(
    operation: &CreateTextBoxV1,
) -> Result<CreateTextBoxPlanV1, CreateTextBoxError> {
    validate_uuid_v7_v1(&operation.node_id).map_err(|_| CreateTextBoxError::InvalidNodeId)?;
    validate_uuid_v7_v1(&operation.story_id).map_err(|_| CreateTextBoxError::InvalidStoryId)?;
    if operation.node_id == operation.story_id {
        return Err(CreateTextBoxError::InvalidStoryId);
    }
    if operation.page_id.is_empty() {
        return Err(CreateTextBoxError::EmptyPageId);
    }
    validate_rect_emu_v1(operation.bounds).map_err(|_| CreateTextBoxError::InvalidBounds)?;
    operation
        .text_preset
        .validate()
        .map_err(CreateTextBoxError::InvalidPreset)?;
    let preset_id = authoring_text_preset_id_v1(&operation.text_preset)
        .map_err(CreateTextBoxError::InvalidPreset)?;

    Ok(CreateTextBoxPlanV1 {
        frame: AuthoredTextFrameV1 {
            node_id: operation.node_id.clone(),
            page_id: operation.page_id.clone(),
            parent_id: operation.page_id.clone(),
            story_id: operation.story_id.clone(),
            bounds: operation.bounds,
            text_preset_id: preset_id.clone(),
            provenance: TextBoxProvenanceV1::AuthorCreated,
        },
        story: AuthoringStorySeedV1 {
            story_id: operation.story_id.clone(),
            text_preset_id: preset_id,
            text_preset: operation.text_preset.clone(),
            initial_text: operation.initial_text.clone(),
            provenance: TextBoxProvenanceV1::AuthorCreated,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        AuthoringCharacterDefaultsV1, AuthoringParagraphAlignmentV1, AuthoringParagraphDefaultsV1,
        font_fingerprint_v1,
    };

    fn preset() -> AuthoringTextPresetV1 {
        AuthoringTextPresetV1::new(
            font_fingerprint_v1(b"pinned-font-bytes").expect("font fingerprint"),
            0,
            152_400,
            AuthoringParagraphDefaultsV1 {
                alignment: AuthoringParagraphAlignmentV1::Left,
                space_before_emu: 0,
                space_after_emu: 0,
            },
            Some(AuthoringCharacterDefaultsV1 {
                bold: false,
                italic: false,
            }),
        )
        .expect("preset")
    }

    fn operation(initial_text: Option<&str>) -> CreateTextBoxV1 {
        CreateTextBoxV1 {
            node_id: "01890f47-0c00-7abc-8def-0123456789ab".to_owned(),
            story_id: "01890f47-0c01-7abc-8def-0123456789ab".to_owned(),
            page_id: "page:1".to_owned(),
            bounds: RectEmuV1 {
                x: 10,
                y: 20,
                width: 3_000_000,
                height: 1_000_000,
            },
            text_preset: preset(),
            initial_text: initial_text.map(str::to_owned),
        }
    }

    #[test]
    fn one_plan_binds_frame_story_page_geometry_and_preset() {
        let op = operation(Some("A\r\nB"));
        let plan = create_textbox_plan_v1(&op).expect("plan");
        assert_eq!(plan.frame.node_id, op.node_id);
        assert_eq!(plan.frame.story_id, op.story_id);
        assert_eq!(plan.frame.parent_id, "page:1");
        assert_eq!(plan.frame.bounds, op.bounds);
        assert_eq!(plan.frame.text_preset_id, plan.story.text_preset_id);
        assert_eq!(plan.story.text_preset, op.text_preset);
        assert_eq!(plan.story.initial_text.as_deref(), Some("A\r\nB"));
        assert_eq!(plan.frame.provenance, TextBoxProvenanceV1::AuthorCreated);
        assert_eq!(plan.story.provenance, TextBoxProvenanceV1::AuthorCreated);
    }

    #[test]
    fn empty_story_seed_stays_empty_without_storage_sentinel() {
        let plan = create_textbox_plan_v1(&operation(None)).expect("plan");
        assert_eq!(plan.story.initial_text, None);
        let json = serde_json::to_string(&plan.story).expect("serialize");
        let lower = json.to_ascii_lowercase();
        assert!(!lower.contains("syid"));
        assert!(!lower.contains("stsh"));
        assert!(!lower.contains("bte"));
        assert!(!lower.contains("terminal_cr"));
    }

    #[test]
    fn replay_and_serialization_preserve_ids_and_preset_binding() {
        let op = operation(Some("Text"));
        let first = create_textbox_plan_v1(&op).expect("first");
        let replay = create_textbox_plan_v1(&op).expect("replay");
        assert_eq!(first, replay);

        let frame_bytes = serde_json::to_vec(&first.frame).expect("frame serialize");
        let frame: AuthoredTextFrameV1 =
            serde_json::from_slice(&frame_bytes).expect("frame deserialize");
        assert_eq!(frame, first.frame);

        let story_bytes = serde_json::to_vec(&first.story).expect("story serialize");
        let story: AuthoringStorySeedV1 =
            serde_json::from_slice(&story_bytes).expect("story deserialize");
        assert_eq!(story, first.story);
    }

    #[test]
    fn bad_ids_bounds_or_preset_fail_closed() {
        let mut bad_node = operation(None);
        bad_node.node_id = "not-a-uuid".to_owned();
        assert_eq!(
            create_textbox_plan_v1(&bad_node),
            Err(CreateTextBoxError::InvalidNodeId)
        );

        let mut bad_story = operation(None);
        bad_story.story_id = "01890f47-0c01-4abc-8def-0123456789ab".to_owned();
        assert_eq!(
            create_textbox_plan_v1(&bad_story),
            Err(CreateTextBoxError::InvalidStoryId)
        );

        let mut same_identity = operation(None);
        same_identity.story_id = same_identity.node_id.clone();
        assert_eq!(
            create_textbox_plan_v1(&same_identity),
            Err(CreateTextBoxError::InvalidStoryId)
        );

        let mut bad_bounds = operation(None);
        bad_bounds.bounds.width = 0;
        assert_eq!(
            create_textbox_plan_v1(&bad_bounds),
            Err(CreateTextBoxError::InvalidBounds)
        );

        let mut bad_preset = operation(None);
        bad_preset.text_preset.font_size_emu = 0;
        assert_eq!(
            create_textbox_plan_v1(&bad_preset),
            Err(CreateTextBoxError::InvalidPreset(
                AuthoringTextPresetError::InvalidFontSizeEmu
            ))
        );
    }
}
