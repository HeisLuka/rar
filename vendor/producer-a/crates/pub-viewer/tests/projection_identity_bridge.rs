use pub_model::{
    Sha256Digest, SourceDerivedIdInput, derive_source_canonical_id,
};
use root_pub_model::{
    derive_pub_node_id_v1 as derive_root_node_id,
    derive_pub_page_id_v1 as derive_root_page_id,
    derive_pub_story_id_v1 as derive_root_story_id,
};

const CARLTON_MARCH_SHA256: &str =
    "bf9cda0f632b5820ab9dbdbe1b838b2a988b2f3fdd69253c22b4fc3aef9f11c3";

fn vendor_id(source_hash: &str, source_object_key: String, semantic_role: &str) -> String {
    let source_hash = source_hash
        .parse::<Sha256Digest>()
        .expect("exact lowercase SHA-256");
    derive_source_canonical_id(SourceDerivedIdInput {
        source_hash: &source_hash,
        adapter_id: "pub-rs",
        source_object_key: &source_object_key,
        semantic_role,
    })
    .expect("vendor source-derived identity")
    .to_string()
}

#[test]
fn carlton_projection_ids_match_root_and_active_vendor_contracts() {
    // Exact March projection controls already proven in PLCCMOB-PROJECTION-01:
    // customer PAGEs 266/361/406 and target frames
    // Qsid49->seq369, Qsid120->seq402, Qsid216->seq435, Qsid218->seq437.
    for page_seq in [266_u32, 361, 406] {
        let root = derive_root_page_id(CARLTON_MARCH_SHA256, page_seq)
            .expect("root PageId");
        let vendor = vendor_id(
            CARLTON_MARCH_SHA256,
            format!("contents/0x2c/seq/{page_seq}"),
            "cdm.page",
        );
        assert_eq!(root, vendor, "PageId contract drift at seq {page_seq}");
    }

    for frame_seq in [369_u32, 402, 435, 437] {
        let root = derive_root_node_id(CARLTON_MARCH_SHA256, frame_seq)
            .expect("root frame NodeId");
        let vendor = vendor_id(
            CARLTON_MARCH_SHA256,
            format!("contents/0x2c/seq/{frame_seq}"),
            "cdm.node",
        );
        assert_eq!(root, vendor, "NodeId contract drift at seq {frame_seq}");
    }

    for qsid in [49_u32, 120, 216, 218] {
        let root = derive_root_story_id(CARLTON_MARCH_SHA256, qsid)
            .expect("root target StoryId");
        let vendor = vendor_id(
            CARLTON_MARCH_SHA256,
            format!("quill/syid/{qsid}"),
            "cdm.story",
        );
        assert_eq!(root, vendor, "StoryId contract drift at Qsid {qsid}");
    }
}

#[test]
fn root_and_vendor_role_separation_remains_identical() {
    let seq = 437_u32;

    let root_page = derive_root_page_id(CARLTON_MARCH_SHA256, seq).expect("root page");
    let root_node = derive_root_node_id(CARLTON_MARCH_SHA256, seq).expect("root node");
    assert_ne!(root_page, root_node);

    let vendor_page = vendor_id(
        CARLTON_MARCH_SHA256,
        format!("contents/0x2c/seq/{seq}"),
        "cdm.page",
    );
    let vendor_node = vendor_id(
        CARLTON_MARCH_SHA256,
        format!("contents/0x2c/seq/{seq}"),
        "cdm.node",
    );

    assert_eq!(root_page, vendor_page);
    assert_eq!(root_node, vendor_node);
    assert_ne!(vendor_page, vendor_node);
}
