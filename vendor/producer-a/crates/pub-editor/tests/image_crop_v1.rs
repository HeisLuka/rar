use pub_editor::{
    EDITOR_PROJECT_VERSION_CURRENT, EDITOR_PROJECT_VERSION_V0_12, EditOperation, EditorError,
    ImageCropStateV1, NodeId,
};

fn node_id() -> NodeId {
    serde_json::from_str("\"20000000-0000-4000-8000-0000000000c1\"").expect("canonical NodeId JSON")
}

fn crop(top: u32, bottom: u32, left: u32, right: u32) -> ImageCropStateV1 {
    ImageCropStateV1 {
        top_raw: Some(top),
        bottom_raw: Some(bottom),
        left_raw: Some(left),
        right_raw: Some(right),
    }
}

#[test]
fn set_image_crop_has_canonical_wire_shape_and_v0_12_fence() {
    assert_eq!(EDITOR_PROJECT_VERSION_CURRENT, EDITOR_PROJECT_VERSION_V0_12);
    let operation = EditOperation::SetImageCrop {
        node_id: node_id(),
        before: crop(1, 2, 3, 4),
        after: crop(5, 6, 7, 8),
    };
    let value = serde_json::to_value(operation).expect("serialize SetImageCrop");
    assert_eq!(value["kind"], "set_image_crop");
    assert_eq!(value["before"]["top_raw"], 1);
    assert_eq!(value["after"]["right_raw"], 8);
}

#[test]
fn image_crop_error_codes_are_stable() {
    let node_id = node_id();
    assert_eq!(
        EditorError::ImageCropUnsupported { node_id }.code(),
        "image_crop_unsupported"
    );
    assert_eq!(
        EditorError::ImageCropNoChange { node_id }.code(),
        "image_crop_no_change"
    );
    assert_eq!(
        EditorError::StaleImageCrop { node_id }.code(),
        "stale_image_crop"
    );
}
