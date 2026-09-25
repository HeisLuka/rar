use pub_editor::{
    EDITOR_PROJECT_VERSION_CURRENT, EDITOR_PROJECT_VERSION_V0_5, EDITOR_PROJECT_VERSION_V0_6,
    EditOperation, EditorError,
    LengthEmu, NodeId, RectEmu,
};

fn node_id() -> NodeId {
    serde_json::from_str("\"20000000-0000-4000-8000-000000000001\"")
        .expect("canonical NodeId JSON")
}

fn rect(x: i64, y: i64, width: i64, height: i64) -> RectEmu {
    RectEmu::new(
        LengthEmu::new(x),
        LengthEmu::new(y),
        LengthEmu::new(width),
        LengthEmu::new(height),
    )
}

#[test]
fn current_project_schema_advances_to_v0_6_without_erasing_v0_5() {
    assert_eq!(EDITOR_PROJECT_VERSION_CURRENT, EDITOR_PROJECT_VERSION_V0_6);
    assert_eq!(EDITOR_PROJECT_VERSION_V0_6, "pub-editor-v0.6");
    assert_eq!(EDITOR_PROJECT_VERSION_V0_5, "pub-editor-v0.5");
}

#[test]
fn resize_node_serializes_as_canonical_bounds_operation() {
    let operation = EditOperation::ResizeNode {
        node_id: node_id(),
        before: rect(0, 0, 127_000, 254_000),
        after: rect(-63_500, -127_000, 254_000, 508_000),
    };
    let value = serde_json::to_value(operation).expect("serialize ResizeNode");
    assert_eq!(value["kind"], "resize_node");
    assert_eq!(value["before"]["width"], 127_000);
    assert_eq!(value["after"]["x"], -63_500);
    assert_eq!(value["after"]["height"], 508_000);
}

#[test]
fn resize_node_error_codes_are_stable() {
    let node_id = node_id();
    assert_eq!(
        EditorError::NodeResizeUnsupported { node_id }.code(),
        "node_resize_unsupported"
    );
    assert_eq!(
        EditorError::NodeResizeNoChange { node_id }.code(),
        "node_resize_no_change"
    );
    assert_eq!(
        EditorError::NodeResizeNoSizeChange { node_id }.code(),
        "node_resize_no_size_change"
    );
    assert_eq!(
        EditorError::NodeResizeNonPositive { node_id }.code(),
        "node_resize_non_positive"
    );
    assert_eq!(
        EditorError::NodeResizeOverflow { node_id }.code(),
        "node_resize_overflow"
    );
    assert_eq!(
        EditorError::StaleNodeResize { node_id }.code(),
        "stale_node_resize"
    );
}
