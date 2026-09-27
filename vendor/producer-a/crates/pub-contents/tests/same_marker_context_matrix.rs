use pub_contents::{
    parse_confirmed_0x2c_directory, parse_confirmed_block, parse_confirmed_chunk_reference,
    ContentsCursor, RawContentsBlockBody, CHUNK_REFERENCE_WIRE_OFFSET,
    CHUNK_REFERENCE_WIRE_PARENT_SEQ_NUM, CHUNK_REFERENCE_WIRE_U16,
};
use pub_core::{RawSpan, StreamPath};

fn span(len: usize) -> RawSpan {
    RawSpan {
        stream: StreamPath("/Synthetic".into()),
        offset: 0,
        len: len as u64,
    }
}

fn reference(fields: &[u8]) -> Vec<u8> {
    let declared = 4u32 + fields.len() as u32;
    let mut bytes = vec![0x00, 0x88];
    bytes.extend_from_slice(&declared.to_le_bytes());
    bytes.extend_from_slice(fields);
    bytes
}

#[test]
fn generic_same_id_block_is_only_a_physical_block() {
    let bytes = [0x02, 0x10, 0x44, 0x00];
    let mut cursor = ContentsCursor::new(StreamPath("/Synthetic".into()), &bytes);
    let block = parse_confirmed_block(&mut cursor).expect("generic field must parse physically");

    assert_eq!(block.id, 0x02);
    assert_eq!(block.block_type, 0x10);
    assert!(matches!(block.body, RawContentsBlockBody::U16 { value: 0x44, .. }));
}

#[test]
fn raw_type_promotion_requires_reference_context_and_wire_0x18() {
    let good = reference(&[0x02, CHUNK_REFERENCE_WIRE_U16, 0x44, 0x00]);
    let dir = parse_confirmed_0x2c_directory(&good, span(good.len())).unwrap();
    let parsed = parse_confirmed_chunk_reference(&good, &dir, 0)
        .unwrap()
        .unwrap();

    assert_eq!(parsed.fields.len(), 1);
    assert_eq!(parsed.raw_types.len(), 1);
    assert_eq!(parsed.raw_types[0].value, 0x44);

    let wrong_wire = reference(&[0x02, 0x10, 0x44, 0x00]);
    let dir = parse_confirmed_0x2c_directory(&wrong_wire, span(wrong_wire.len())).unwrap();
    let parsed = parse_confirmed_chunk_reference(&wrong_wire, &dir, 0)
        .unwrap()
        .unwrap();

    assert_eq!(parsed.fields.len(), 1);
    assert!(parsed.raw_types.is_empty());
}

#[test]
fn chunk_offset_promotion_requires_id_0x04_and_wire_0xb8() {
    let good = reference(&[0x04, CHUNK_REFERENCE_WIRE_OFFSET, 0x34, 0x12, 0x00, 0x00]);
    let dir = parse_confirmed_0x2c_directory(&good, span(good.len())).unwrap();
    let parsed = parse_confirmed_chunk_reference(&good, &dir, 0)
        .unwrap()
        .unwrap();
    assert_eq!(parsed.chunk_offsets.len(), 1);
    assert_eq!(parsed.chunk_offsets[0].value, 0x1234);

    let wrong_wire = reference(&[0x04, 0x20, 0x34, 0x12, 0x00, 0x00]);
    let dir = parse_confirmed_0x2c_directory(&wrong_wire, span(wrong_wire.len())).unwrap();
    let parsed = parse_confirmed_chunk_reference(&wrong_wire, &dir, 0)
        .unwrap()
        .unwrap();

    assert_eq!(parsed.fields.len(), 1);
    assert!(parsed.chunk_offsets.is_empty());
}

#[test]
fn parent_seq_promotion_requires_id_0x05_and_wire_0x68() {
    let good = reference(&[
        0x05,
        CHUNK_REFERENCE_WIRE_PARENT_SEQ_NUM,
        0x07,
        0x00,
        0x00,
        0x00,
    ]);
    let dir = parse_confirmed_0x2c_directory(&good, span(good.len())).unwrap();
    let parsed = parse_confirmed_chunk_reference(&good, &dir, 0)
        .unwrap()
        .unwrap();
    assert_eq!(parsed.parent_seq_nums.len(), 1);
    assert_eq!(parsed.parent_seq_nums[0].value, 7);

    let wrong_wire = reference(&[0x05, 0x20, 0x07, 0x00, 0x00, 0x00]);
    let dir = parse_confirmed_0x2c_directory(&wrong_wire, span(wrong_wire.len())).unwrap();
    let parsed = parse_confirmed_chunk_reference(&wrong_wire, &dir, 0)
        .unwrap()
        .unwrap();

    assert_eq!(parsed.fields.len(), 1);
    assert!(parsed.parent_seq_nums.is_empty());
}

#[test]
fn same_supported_wire_with_wrong_id_stays_unpromoted() {
    let bytes = reference(&[
        0x06, CHUNK_REFERENCE_WIRE_U16, 0x44, 0x00,
        0x07, CHUNK_REFERENCE_WIRE_OFFSET, 0x34, 0x12, 0x00, 0x00,
        0x08, CHUNK_REFERENCE_WIRE_PARENT_SEQ_NUM, 0x07, 0x00, 0x00, 0x00,
    ]);
    let dir = parse_confirmed_0x2c_directory(&bytes, span(bytes.len())).unwrap();
    let parsed = parse_confirmed_chunk_reference(&bytes, &dir, 0)
        .unwrap()
        .unwrap();

    assert_eq!(parsed.fields.len(), 3);
    assert!(parsed.raw_types.is_empty());
    assert!(parsed.chunk_offsets.is_empty());
    assert!(parsed.parent_seq_nums.is_empty());
}
