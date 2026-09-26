use chaptera_caret_layout_feed::build_caret_map_from_shaped_flow_v1;
use chaptera_text_caret_map_adapter::ResolvedTextCaretMapV1;
use pub_editor::EditorSession;
use pub_layout::{
    BoundedLayoutEnvironment, BoundedShapedFlowRuntime, BoundedShapedFlowScene,
    BoundedShapingRuntime, font_fingerprint_sha256, project_bounded, resolve_bounded_shaped_flow,
};
use pub_model::{LengthEmu, StoryId};
use std::fmt;

pub const DESKTOP_SHAPED_FLOW_RUNTIME_V1: &str = "chaptera.desktop-shaped-flow-runtime.v1";

#[derive(Debug, Clone, Copy)]
pub struct ExplicitDesktopFontResourceV1<'a> {
    pub resource_id: &'a str,
    pub expected_sha256: &'a str,
    pub face_index: u32,
    pub font_size_emu: LengthEmu,
    pub line_height_emu: LengthEmu,
    pub bytes: &'a [u8],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DesktopStoryLayoutV1 {
    pub layout_revision_id: String,
    pub story_id: StoryId,
    pub story_scalar_len: u32,
    pub font_fingerprint_sha256: String,
    pub shaped_flow: BoundedShapedFlowScene,
    pub caret_map: ResolvedTextCaretMapV1,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DesktopShapedFlowRuntimeError {
    pub code: &'static str,
    pub message: String,
}

impl DesktopShapedFlowRuntimeError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl fmt::Display for DesktopShapedFlowRuntimeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for DesktopShapedFlowRuntimeError {}

pub fn validate_explicit_font_resource_v1(
    font: &ExplicitDesktopFontResourceV1<'_>,
) -> Result<String, DesktopShapedFlowRuntimeError> {
    if font.resource_id.is_empty() {
        return Err(DesktopShapedFlowRuntimeError::new(
            "font_resource_missing",
            "explicit Desktop font resource_id is required",
        ));
    }
    if font.bytes.is_empty() {
        return Err(DesktopShapedFlowRuntimeError::new(
            "font_resource_missing",
            "explicit Desktop font bytes are required",
        ));
    }
    if font.font_size_emu.get() <= 0 {
        return Err(DesktopShapedFlowRuntimeError::new(
            "invalid_font_size",
            "font_size_emu must be positive",
        ));
    }
    if font.line_height_emu.get() <= 0 {
        return Err(DesktopShapedFlowRuntimeError::new(
            "invalid_line_height",
            "line_height_emu must be positive",
        ));
    }

    let actual = font_fingerprint_sha256(font.bytes);
    if font.expected_sha256.is_empty() || actual != font.expected_sha256 {
        return Err(DesktopShapedFlowRuntimeError::new(
            "font_fingerprint_mismatch",
            format!(
                "explicit Desktop font fingerprint mismatch: expected={} actual={actual}",
                font.expected_sha256
            ),
        ));
    }
    Ok(actual)
}

pub fn build_current_story_layout_v1(
    editor: &EditorSession,
    story_id: StoryId,
    layout_revision_id: &str,
    font: &ExplicitDesktopFontResourceV1<'_>,
) -> Result<DesktopStoryLayoutV1, DesktopShapedFlowRuntimeError> {
    if layout_revision_id.is_empty() {
        return Err(DesktopShapedFlowRuntimeError::new(
            "invalid_layout_revision",
            "layout_revision_id is required",
        ));
    }

    let story = editor.graph().stories.get(&story_id).ok_or_else(|| {
        DesktopShapedFlowRuntimeError::new(
            "story_missing",
            "requested Story is absent from current EditorSession graph",
        )
    })?;
    let story_scalar_len = u32::try_from(story.text.chars().count()).map_err(|_| {
        DesktopShapedFlowRuntimeError::new(
            "story_extent_overflow",
            "current Story scalar length exceeds the V1 u32 domain",
        )
    })?;

    let fingerprint = validate_explicit_font_resource_v1(font)?;
    let authoring =
        pub_viewer::bounded_authoring_slice_from_resolved(editor.graph()).map_err(|error| {
            DesktopShapedFlowRuntimeError::new(
                "authoring_projection_failed",
                format!("resolved graph could not enter bounded layout projection: {error}"),
            )
        })?;
    let projection = project_bounded(authoring);

    let runtime = BoundedShapedFlowRuntime {
        shaping: BoundedShapingRuntime {
            layout: BoundedLayoutEnvironment {
                engine_revision: DESKTOP_SHAPED_FLOW_RUNTIME_V1.to_owned(),
                font_set_fingerprint: fingerprint.clone(),
                resource_fingerprint: font.resource_id.to_owned(),
            },
            face_index: font.face_index,
            font_size_emu: font.font_size_emu,
            font_bytes: font.bytes,
        },
        line_height: font.line_height_emu,
    };

    let shaped_flow = resolve_bounded_shaped_flow(&projection, &runtime).map_err(|error| {
        DesktopShapedFlowRuntimeError::new(
            "shaped_flow_failed",
            format!("authoritative bounded shaped flow failed: {error}"),
        )
    })?;

    let caret_map = build_caret_map_from_shaped_flow_v1(
        &shaped_flow,
        layout_revision_id,
        story_id,
        story_scalar_len,
    )
    .map_err(|error| {
        DesktopShapedFlowRuntimeError::new(
            "caret_feed_failed",
            format!("shaped-flow caret projection failed: {error}"),
        )
    })?;

    Ok(DesktopStoryLayoutV1 {
        layout_revision_id: layout_revision_id.to_owned(),
        story_id,
        story_scalar_len,
        font_fingerprint_sha256: fingerprint,
        shaped_flow,
        caret_map,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use pub_editor::{Sha256Digest, open_mature_0x2c_editor};
    use pub_model::EMU_PER_POINT;
    use sha2::{Digest, Sha256};
    use std::{env, fs};

    fn test_font() -> ExplicitDesktopFontResourceV1<'static> {
        let bytes = font_test_data::NOTOSERIF_AUTOHINT_SHAPING;
        let fingerprint = font_fingerprint_sha256(bytes);
        let leaked = Box::leak(fingerprint.into_boxed_str());
        ExplicitDesktopFontResourceV1 {
            resource_id: "dev-test:noto-serif-autohint-shaping",
            expected_sha256: leaked,
            face_index: 0,
            font_size_emu: LengthEmu::new(10 * EMU_PER_POINT),
            line_height_emu: LengthEmu::new(12 * EMU_PER_POINT),
            bytes,
        }
    }

    #[test]
    fn explicit_font_resource_rejects_fingerprint_mismatch() {
        let mut font = test_font();
        font.expected_sha256 = "00";
        let error = validate_explicit_font_resource_v1(&font).unwrap_err();
        assert_eq!(error.code, "font_fingerprint_mismatch");
    }

    #[test]
    fn real_sample_newsletter_current_story_builds_deterministically_and_rebinds_after_edit() {
        let Some(path) = env::var_os("CHAPTERA_SAMPLE_NEWSLETTER") else {
            eprintln!(
                "CHAPTERA_SAMPLE_NEWSLETTER not set; dedicated real-fixture gate owns this test"
            );
            return;
        };

        let bytes = fs::read(path).expect("read pinned SampleNewsletter");
        let digest = Sha256::digest(&bytes);
        let mut digest_bytes = [0_u8; 32];
        digest_bytes.copy_from_slice(&digest);
        let source_hash = Sha256Digest::from_bytes(digest_bytes);
        let mut editor = open_mature_0x2c_editor(&bytes, source_hash)
            .expect("open real SampleNewsletter editor");
        let font = test_font();

        let story_ids = editor.graph().stories.keys().copied().collect::<Vec<_>>();
        let (story_id, before_layout) = story_ids
            .into_iter()
            .filter(|story_id| editor.can_replace_story_text(*story_id).is_ok())
            .find_map(|story_id| {
                build_current_story_layout_v1(&editor, story_id, "layout:before", &font)
                    .ok()
                    .filter(|layout| !layout.caret_map.lines.is_empty())
                    .map(|layout| (story_id, layout))
            })
            .expect(
                "real fixture should expose one editable Story with authoritative shaped lines",
            );

        let deterministic =
            build_current_story_layout_v1(&editor, story_id, "layout:before", &font)
                .expect("repeat current Story layout");
        assert_eq!(before_layout, deterministic);
        assert_eq!(before_layout.caret_map.layout_revision_id, "layout:before");
        assert!(!before_layout.caret_map.caret_stops.is_empty());

        editor
            .replace_story_range(story_id, 0, 0, "", "X")
            .expect("insert one scalar through existing EditorSession authority");
        let after_layout = build_current_story_layout_v1(&editor, story_id, "layout:after", &font)
            .expect("rebuild shaped flow after accepted Story edit");
        assert_eq!(
            after_layout.story_scalar_len,
            before_layout.story_scalar_len + 1
        );
        assert_eq!(after_layout.caret_map.layout_revision_id, "layout:after");
        assert_eq!(editor.source_hash(), source_hash);

        editor.undo().expect("undo real Story insertion");
        let restored = build_current_story_layout_v1(&editor, story_id, "layout:undo", &font)
            .expect("rebuild shaped flow after Undo");
        assert_eq!(restored.story_scalar_len, before_layout.story_scalar_len);
        assert_eq!(editor.source_hash(), source_hash);
    }
}
