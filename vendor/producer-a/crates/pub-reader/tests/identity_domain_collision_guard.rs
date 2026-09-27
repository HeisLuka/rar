use pub_contents::{ContentsCursor, parse_confirmed_block, parse_confirmed_oid_identity_payload};
use pub_core::StreamPath;
use pub_model::Sha256Digest;
use pub_reader::{
    contents_object_key, derive_pub_document_id, derive_pub_node_id, derive_pub_page_id,
    derive_pub_story_id, quill_story_object_key,
};

fn source_hash() -> Sha256Digest {
    let mut bytes = [0u8; 32];
    for (index, byte) in bytes.iter_mut().enumerate() {
        *byte = u8::try_from(index).expect("0..31 fits u8");
    }
    Sha256Digest::from_bytes(bytes)
}

fn oid_payload(value: u32) -> pub_contents::OidIdentityPayload {
    let mut bytes = vec![0x0D, 0x28];
    bytes.extend_from_slice(&value.to_le_bytes());
    bytes.extend_from_slice(&value.to_le_bytes());
    let mut cursor = ContentsCursor::new(StreamPath("/Synthetic".into()), &bytes);
    let block = parse_confirmed_block(&mut cursor).expect("synthetic fixed8 block must parse");
    parse_confirmed_oid_identity_payload(block).expect("synthetic Oid payload must parse")
}

#[test]
fn equal_contents_seq_and_quill_syid_do_not_share_canonical_id() {
    let hash = source_hash();
    let numeric = 7u32;

    let node = derive_pub_node_id(&hash, numeric).expect("node id");
    let story = derive_pub_story_id(&hash, numeric).expect("story id");

    assert_ne!(node.as_ref(), story.as_ref());
    assert_ne!(contents_object_key(numeric), quill_story_object_key(numeric));
}

#[test]
fn one_contents_seq_is_role_separated_for_document_page_and_node() {
    let hash = source_hash();
    let numeric = 7u32;

    let document = derive_pub_document_id(&hash, numeric).expect("document id");
    let page = derive_pub_page_id(&hash, numeric).expect("page id");
    let node = derive_pub_node_id(&hash, numeric).expect("node id");

    assert_ne!(document.as_ref(), page.as_ref());
    assert_ne!(document.as_ref(), node.as_ref());
    assert_ne!(page.as_ref(), node.as_ref());
}

#[test]
fn equal_oid_scalar_remains_a_physical_observation_not_a_join_key() {
    let hash = source_hash();
    let numeric = 7u32;
    let oid = oid_payload(numeric);

    assert_eq!(oid.dword0, numeric);
    assert_eq!(oid.dword1, numeric);

    let node = derive_pub_node_id(&hash, numeric).expect("node id");
    let story = derive_pub_story_id(&hash, numeric).expect("story id");

    // The Oid parser exposes only its physical words and source span. There is
    // deliberately no conversion from these words into NodeId or StoryId.
    assert_ne!(node.as_ref(), story.as_ref());
}

#[test]
fn changing_only_semantic_role_changes_source_derived_identity() {
    let hash = source_hash();
    let numeric = 7u32;

    let document = derive_pub_document_id(&hash, numeric).expect("document id");
    let page = derive_pub_page_id(&hash, numeric).expect("page id");
    let node = derive_pub_node_id(&hash, numeric).expect("node id");

    let ids = [document.as_ref(), page.as_ref(), node.as_ref()];
    assert!(ids[0] != ids[1] && ids[0] != ids[2] && ids[1] != ids[2]);
}
