use pub_contents::{
    BLOCK_TYPE_CONTAINER_88, CONTENTS_PACKED_FIELD_ID_MAX, ContentsCursor, PackedFieldTagError,
    RawContentsBlockBody, decode_packed_field_tag, encode_packed_field_tag, parse_confirmed_block,
};
use pub_core::{RawSpan, StreamPath};

#[test]
fn exact_evidence_vectors_roundtrip() {
    let vectors = [
        (0x0213, 0x08, [0x13, 0x0A]),
        (0x0224, 0x88, [0x24, 0x8A]),
        (0x0206, 0x80, [0x06, 0x82]),
        (0x0257, 0x88, [0x57, 0x8A]),
    ];

    for (field_id, wire_type, raw_tag) in vectors {
        assert_eq!(
            encode_packed_field_tag(field_id, wire_type).expect("evidence vector must encode"),
            raw_tag
        );
        assert_eq!(decode_packed_field_tag(raw_tag), (field_id, wire_type));
    }

    assert_eq!(CONTENTS_PACKED_FIELD_ID_MAX, 0x07ff);
    assert_eq!(
        encode_packed_field_tag(0x0800, 0x20),
        Err(PackedFieldTagError::FieldIdOutOfRange { field_id: 0x0800 })
    );
    assert_eq!(
        encode_packed_field_tag(0x0024, 0x8A),
        Err(PackedFieldTagError::WireTypeNotNormalized { wire_type: 0x8A })
    );
}

#[test]
fn production_parser_keeps_extended_id_and_exact_raw_tag() {
    let bytes = [0x24, 0x8A, 0x04, 0x00, 0x00, 0x00];
    let mut cursor = ContentsCursor::new(StreamPath("/Contents".into()), &bytes);

    let block = parse_confirmed_block(&mut cursor).expect("extended field must parse");

    assert_eq!(block.id, 0x0224);
    assert_eq!(block.block_type, BLOCK_TYPE_CONTAINER_88);
    assert_eq!(block.raw_tag, [0x24, 0x8A]);
    assert_eq!(
        block.tag_source,
        RawSpan {
            stream: StreamPath("/Contents".into()),
            offset: 0,
            len: 2,
        }
    );
    assert!(matches!(
        block.body,
        RawContentsBlockBody::Container {
            declared_length: 4,
            ..
        }
    ));
    assert_eq!(cursor.position(), bytes.len());
}
