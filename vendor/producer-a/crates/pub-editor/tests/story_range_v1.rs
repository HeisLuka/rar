use pub_editor::{EditOperation, StoryId, story_state_id_v1};

fn story_id() -> StoryId {
    serde_json::from_str("\"10000000-0000-4000-8000-000000000001\"")
        .expect("canonical StoryId JSON")
}

#[test]
fn story_state_id_matches_canonical_python_v1_law() {
    assert_eq!(
        story_state_id_v1(story_id(), "Aé😀\r"),
        "sha256:b8b9f4a2bbb28f5949d8933a76ae0010b96c0f5afdca2d2f092f659021199533"
    );
}

#[test]
fn replace_story_range_serializes_to_admitted_v0_4_wire() {
    let operation = EditOperation::ReplaceStoryRange {
        story_id: story_id(),
        start_scalar: 1,
        end_scalar: 2,
        expected_before: "x".to_owned(),
        replacement_text: "ChapteraV0".to_owned(),
        before_story_state_id:
            "sha256:1111111111111111111111111111111111111111111111111111111111111111".to_owned(),
        after_story_state_id:
            "sha256:2222222222222222222222222222222222222222222222222222222222222222".to_owned(),
    };
    let value = serde_json::to_value(operation).expect("serialize range operation");
    assert_eq!(value["kind"], "replace_story_range");
    assert_eq!(value["start_scalar"], 1);
    assert_eq!(value["end_scalar"], 2);
    assert_eq!(value["replacement_text"], "ChapteraV0");
    assert_eq!(
        value["before_story_state_id"],
        "sha256:1111111111111111111111111111111111111111111111111111111111111111"
    );
}
