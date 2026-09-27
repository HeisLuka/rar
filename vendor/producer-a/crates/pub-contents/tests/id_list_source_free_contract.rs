use pub_contents::{
    BLOCK_TYPE_CONTAINER_A0, BLOCK_TYPE_HANDLE_U32, BlockReadError, ContentsCursor,
    RawContentsBlockBody, parse_confirmed_block,
};
use pub_core::StreamPath;

fn parse_exact_u32_list(
    bytes: &[u8],
    expected_entries: usize,
    expected_id: u16,
    expected_wire: u8,
) -> Result<Vec<u32>, String> {
    let mut cursor = ContentsCursor::new(StreamPath("/Synthetic".into()), bytes);
    let mut values = Vec::new();

    while cursor.remaining() > 0 {
        let block = parse_confirmed_block(&mut cursor).map_err(|error| format!("{error:?}"))?;
        if block.id != expected_id {
            return Err(format!(
                "entry id mismatch: expected 0x{expected_id:03X}, got 0x{:03X}",
                block.id
            ));
        }
        if block.block_type != expected_wire {
            return Err(format!(
                "entry wire mismatch: expected 0x{expected_wire:02X}, got 0x{:02X}",
                block.block_type
            ));
        }

        match block.body {
            RawContentsBlockBody::U32 { value, .. } => values.push(value),
            other => return Err(format!("entry body is not u32: {other:?}")),
        }
    }

    if values.len() != expected_entries {
        return Err(format!(
            "entry count mismatch: expected {expected_entries}, got {}",
            values.len()
        ));
    }
    Ok(values)
}

#[test]
fn repeated_handle_u32_entries_preserve_order_and_values() {
    let bytes = [
        0x00, 0x70, 0x11, 0x00, 0x00, 0x00,
        0x00, 0x70, 0x22, 0x00, 0x00, 0x00,
        0x00, 0x70, 0x33, 0x00, 0x00, 0x00,
    ];

    let values = parse_exact_u32_list(&bytes, 3, 0, BLOCK_TYPE_HANDLE_U32)
        .expect("source-free repeated handle list must parse");

    assert_eq!(values, vec![0x11, 0x22, 0x33]);
}

#[test]
fn same_entry_id_with_different_wire_is_not_promoted_to_same_contract() {
    let bytes = [0x00, 0x20, 0x11, 0x00, 0x00, 0x00];

    let error = parse_exact_u32_list(&bytes, 1, 0, BLOCK_TYPE_HANDLE_U32)
        .expect_err("u32 and handle-u32 wires must remain distinct");

    assert!(error.contains("entry wire mismatch"));
}

#[test]
fn explicit_count_mismatch_fails_closed() {
    let bytes = [
        0x00, 0x70, 0x11, 0x00, 0x00, 0x00,
        0x00, 0x70, 0x22, 0x00, 0x00, 0x00,
    ];

    let error = parse_exact_u32_list(&bytes, 3, 0, BLOCK_TYPE_HANDLE_U32)
        .expect_err("an external list count contract must fail on a short list");

    assert!(error.contains("entry count mismatch"));
}

#[test]
fn truncated_entry_does_not_advance_the_shared_parser_cursor() {
    let bytes = [0x00, 0x70, 0x11, 0x22];
    let mut cursor = ContentsCursor::new(StreamPath("/Synthetic".into()), &bytes);

    assert_eq!(cursor.position(), 0);
    let error = parse_confirmed_block(&mut cursor).expect_err("truncated u32 must fail");

    assert!(matches!(error, BlockReadError::Contents(_)));
    assert_eq!(cursor.position(), 0);
}

#[test]
fn unsupported_wire_does_not_advance_the_shared_parser_cursor() {
    let bytes = [0x00, 0xC0];
    let mut cursor = ContentsCursor::new(StreamPath("/Synthetic".into()), &bytes);

    assert_eq!(cursor.position(), 0);
    let error = parse_confirmed_block(&mut cursor).expect_err("unsupported wire must fail");

    assert!(matches!(error, BlockReadError::UnsupportedType { .. }));
    assert_eq!(cursor.position(), 0);
}

#[test]
fn bounded_nested_container_keeps_child_list_inside_declared_span() {
    let bytes = [
        0x02, 0xA0, 0x10, 0x00, 0x00, 0x00,
        0x00, 0x70, 0x11, 0x00, 0x00, 0x00,
        0x00, 0x70, 0x22, 0x00, 0x00, 0x00,
        0xEE, 0xEE, 0xEE, 0xEE,
    ];
    let mut cursor = ContentsCursor::new(StreamPath("/Synthetic".into()), &bytes);
    let outer = parse_confirmed_block(&mut cursor).expect("outer container must parse");

    assert_eq!(outer.id, 0x02);
    assert_eq!(outer.block_type, BLOCK_TYPE_CONTAINER_A0);

    let RawContentsBlockBody::Container { content_source, .. } = outer.body else {
        panic!("outer A0 must expose container body");
    };
    assert_eq!(content_source.offset, 6);
    assert_eq!(content_source.len, 12);

    let start = usize::try_from(content_source.offset).unwrap();
    let end = start + usize::try_from(content_source.len).unwrap();
    let values = parse_exact_u32_list(
        &bytes[start..end],
        2,
        0,
        BLOCK_TYPE_HANDLE_U32,
    )
    .expect("nested list inside exact declared span must parse");

    assert_eq!(values, vec![0x11, 0x22]);
    assert_eq!(cursor.position(), 18);
    assert_eq!(&bytes[18..], &[0xEE, 0xEE, 0xEE, 0xEE]);
}

#[test]
fn container_declared_length_underflow_is_rejected() {
    let bytes = [0x02, 0xA0, 0x03, 0x00, 0x00, 0x00];
    let mut cursor = ContentsCursor::new(StreamPath("/Synthetic".into()), &bytes);

    let error = parse_confirmed_block(&mut cursor).expect_err("declared length < 4 must fail");
    assert!(matches!(error, BlockReadError::InvalidDeclaredLength { .. }));
    assert_eq!(cursor.position(), 0);
}

#[test]
fn container_declared_length_past_available_bytes_is_rejected() {
    let bytes = [
        0x02, 0xA0, 0x10, 0x00, 0x00, 0x00,
        0x00, 0x70, 0x11, 0x00,
    ];
    let mut cursor = ContentsCursor::new(StreamPath("/Synthetic".into()), &bytes);

    let error = parse_confirmed_block(&mut cursor).expect_err("truncated declared body must fail");
    assert!(matches!(error, BlockReadError::Contents(_)));
    assert_eq!(cursor.position(), 0);
}
