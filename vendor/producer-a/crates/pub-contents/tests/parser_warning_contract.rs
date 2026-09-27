use pub_contents::{
    BlockReadError, ChunkReadError, ChunkReferenceReadError, ContentsCursor, DirectoryReadError,
    parse_confirmed_0x2c_chunk, parse_confirmed_0x2c_directory, parse_confirmed_block,
    parse_confirmed_chunk_reference,
};
use pub_core::{RawSpan, StreamPath};

fn span(len: u64) -> RawSpan {
    RawSpan {
        stream: StreamPath("/Synthetic".into()),
        offset: 0,
        len,
    }
}

#[test]
fn unsupported_block_wire_has_dedicated_error_and_preserves_cursor() {
    let bytes = [0x00, 0xC0];
    let mut cursor = ContentsCursor::new(StreamPath("/Synthetic".into()), &bytes);
    let error = parse_confirmed_block(&mut cursor).expect_err("unsupported wire must fail");

    assert!(matches!(error, BlockReadError::UnsupportedType { block_type: 0xC0, offset: 0 }));
    assert_eq!(cursor.position(), 0);
}

#[test]
fn truncated_u32_is_not_reported_as_unsupported_wire() {
    let bytes = [0x00, 0x20, 0x11, 0x22];
    let mut cursor = ContentsCursor::new(StreamPath("/Synthetic".into()), &bytes);
    let error = parse_confirmed_block(&mut cursor).expect_err("truncated u32 must fail");

    assert!(matches!(error, BlockReadError::Contents(_)));
    assert_eq!(cursor.position(), 0);
}

#[test]
fn invalid_container_length_has_dedicated_error() {
    let bytes = [0x00, 0x88, 0x03, 0x00, 0x00, 0x00];
    let mut cursor = ContentsCursor::new(StreamPath("/Synthetic".into()), &bytes);
    let error = parse_confirmed_block(&mut cursor).expect_err("declared length < 4 must fail");

    assert!(matches!(
        error,
        BlockReadError::InvalidDeclaredLength {
            block_type: 0x88,
            offset: 0,
            declared_length: 3
        }
    ));
    assert_eq!(cursor.position(), 0);
}

#[test]
fn chunk_declared_range_oob_is_distinct_from_block_error() {
    let bytes = [0x20, 0x00, 0x00, 0x00, 0, 0, 0, 0];
    let error = parse_confirmed_0x2c_chunk(StreamPath("/Synthetic".into()), &bytes, 0)
        .expect_err("chunk range must fit stream");

    assert!(matches!(
        error,
        ChunkReadError::DeclaredRangeOutOfBounds {
            offset: 0,
            declared_length: 32,
            stream_len: 8
        }
    ));
}

#[test]
fn directory_wrong_slot_id_is_distinct_from_wire_parse_failure() {
    let bytes = [0x01, 0x78];
    let error = parse_confirmed_0x2c_directory(&bytes, span(bytes.len() as u64))
        .expect_err("directory slot id must be zero");

    assert!(matches!(error, DirectoryReadError::UnexpectedSlotId { offset: 0, id: 1 }));
}

#[test]
fn directory_wrong_supported_wire_is_context_error() {
    let bytes = [0x00, 0x20, 0x01, 0x00, 0x00, 0x00];
    let error = parse_confirmed_0x2c_directory(&bytes, span(bytes.len() as u64))
        .expect_err("generic u32 block is not a directory slot");

    assert!(matches!(
        error,
        DirectoryReadError::UnexpectedSlotType {
            offset: 0,
            block_type: 0x20
        }
    ));
}

#[test]
fn reference_context_reports_unsupported_reference_wire_separately() {
    let bytes = [
        0x00, 0x88, 0x06, 0x00, 0x00, 0x00,
        0x02, 0xC0,
    ];
    let directory = parse_confirmed_0x2c_directory(&bytes, span(bytes.len() as u64))
        .expect("directory envelope must parse");
    let error = parse_confirmed_chunk_reference(&bytes, &directory, 0)
        .expect_err("reference-only wire taxonomy must fail locally");

    assert!(matches!(
        error,
        ChunkReferenceReadError::UnsupportedReferenceWireType {
            block_type: 0xC0,
            offset: 6
        }
    ));
}

#[test]
fn reference_out_of_range_is_not_collapsed_into_generic_parse_error() {
    let bytes = [0x00, 0x78];
    let directory = parse_confirmed_0x2c_directory(&bytes, span(bytes.len() as u64))
        .expect("single empty directory slot must parse");
    let error = parse_confirmed_chunk_reference(&bytes, &directory, 1)
        .expect_err("out of range seqNum must be explicit");

    assert!(matches!(
        error,
        ChunkReferenceReadError::SlotOutOfRange {
            seq_num: 1,
            slot_count: 1
        }
    ));
}
