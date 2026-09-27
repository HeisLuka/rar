use pub_contents::{
    parse_0x2c_header, parse_legacy_0x22_formatting_descriptor, parse_legacy_0x22_table_catalog,
    ContentsFamily, ContentsReadError, Legacy0x22ReadError, Legacy0x22TableCatalogReadError,
    CONTENTS_0X22_MAGIC, CONTENTS_0X2C_MAGIC,
};
use pub_core::StreamPath;

fn mature_header_with_marker(marker: u16) -> Vec<u8> {
    let mut bytes = vec![0u8; 64];
    bytes[0..4].copy_from_slice(&CONTENTS_0X2C_MAGIC);
    bytes[12..14].copy_from_slice(&0x001Au16.to_le_bytes());
    bytes[0x1A..0x1E].copy_from_slice(&40u32.to_le_bytes());
    bytes[40..42].copy_from_slice(&marker.to_le_bytes());
    bytes
}

fn legacy_header_with_marker(marker: u16) -> Vec<u8> {
    let mut bytes = vec![0u8; 0x800];
    bytes[0..4].copy_from_slice(&CONTENTS_0X22_MAGIC);
    bytes[0x12..0x16].copy_from_slice(&0x22u32.to_le_bytes());
    let descriptor = 0x22 + 14;
    bytes[descriptor..descriptor + 4].copy_from_slice(&0x50u32.to_le_bytes());
    bytes[descriptor + 4..descriptor + 8].copy_from_slice(&0x50u32.to_le_bytes());
    bytes[descriptor + 8..descriptor + 10].copy_from_slice(&0u16.to_le_bytes());
    bytes[descriptor + 10..descriptor + 12].copy_from_slice(&0u16.to_le_bytes());
    bytes[descriptor + 12..descriptor + 14].copy_from_slice(&0u16.to_le_bytes());
    bytes[descriptor + 14..descriptor + 16].copy_from_slice(&0u16.to_le_bytes());
    bytes[0x16..0x1A].copy_from_slice(&0x100u32.to_le_bytes());
    bytes[0x100..0x102].copy_from_slice(&0u16.to_le_bytes());
    bytes[0x120..0x122].copy_from_slice(&marker.to_le_bytes());
    bytes
}

#[test]
fn mature_header_rejects_legacy_family_before_interpreting_mature_fields() {
    let mut bytes = mature_header_with_marker(0x0044);
    bytes[0..4].copy_from_slice(&CONTENTS_0X22_MAGIC);

    assert!(matches!(
        parse_0x2c_header(StreamPath("/Synthetic".into()), &bytes),
        Err(ContentsReadError::UnexpectedFamily {
            expected: ContentsFamily::Family0x2c,
            found: ContentsFamily::Family0x22
        })
    ));
}

#[test]
fn legacy_formatting_rejects_mature_family_before_legacy_descriptor_use() {
    let mut bytes = legacy_header_with_marker(0x0044);
    bytes[0..4].copy_from_slice(&CONTENTS_0X2C_MAGIC);

    assert!(matches!(
        parse_legacy_0x22_formatting_descriptor(StreamPath("/Synthetic".into()), &bytes),
        Err(Legacy0x22ReadError::UnexpectedFamily(ContentsFamily::Family0x2c))
    ));
}

#[test]
fn legacy_table_catalog_rejects_mature_family_before_legacy_directory_use() {
    let mut bytes = legacy_header_with_marker(0x0044);
    bytes[0..4].copy_from_slice(&CONTENTS_0X2C_MAGIC);

    assert!(matches!(
        parse_legacy_0x22_table_catalog(StreamPath("/Synthetic".into()), &bytes),
        Err(Legacy0x22TableCatalogReadError::UnexpectedFamily(
            ContentsFamily::Family0x2c
        ))
    ));
}

#[test]
fn same_numeric_marker_does_not_select_a_cross_generation_decoder() {
    let marker = 0x0044u16;
    let legacy = legacy_header_with_marker(marker);
    let mature = mature_header_with_marker(marker);

    assert_eq!(
        u16::from_le_bytes([legacy[0x120], legacy[0x121]]),
        u16::from_le_bytes([mature[40], mature[41]])
    );

    let mature_on_legacy = {
        let mut bytes = legacy.clone();
        bytes.resize(0x800, 0);
        parse_0x2c_header(StreamPath("/LegacyAsMature".into()), &bytes)
    };
    assert!(matches!(
        mature_on_legacy,
        Err(ContentsReadError::UnexpectedFamily {
            expected: ContentsFamily::Family0x2c,
            found: ContentsFamily::Family0x22
        })
    ));

    let legacy_on_mature = parse_legacy_0x22_formatting_descriptor(
        StreamPath("/MatureAsLegacy".into()),
        &mature,
    );
    assert!(matches!(
        legacy_on_mature,
        Err(Legacy0x22ReadError::UnexpectedFamily(
            ContentsFamily::Family0x2c
        ))
    ));
}
