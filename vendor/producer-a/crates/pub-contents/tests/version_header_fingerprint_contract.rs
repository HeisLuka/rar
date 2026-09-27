use pub_contents::{
    detect_family, parse_preamble, ContentsFamily, ContentsReadError, CONTENTS_0X22_MAGIC,
    CONTENTS_0X2C_MAGIC,
};
use pub_core::StreamPath;

fn header(magic: [u8; 4], revision: u16) -> Vec<u8> {
    let mut bytes = vec![0u8; 14];
    bytes[0..4].copy_from_slice(&magic);
    bytes[12..14].copy_from_slice(&revision.to_le_bytes());
    bytes
}

#[test]
fn exact_family_magic_is_required() {
    assert_eq!(
        detect_family(&CONTENTS_0X22_MAGIC),
        Ok(ContentsFamily::Family0x22)
    );
    assert_eq!(
        detect_family(&CONTENTS_0X2C_MAGIC),
        Ok(ContentsFamily::Family0x2c)
    );
    assert!(matches!(
        detect_family(&[0xE8, 0xAC, 0x2C, 0x01]),
        Err(ContentsReadError::UnsupportedMagic(_))
    ));
}

#[test]
fn short_family_marker_fails_closed() {
    assert!(matches!(
        detect_family(&[0xE8, 0xAC, 0x2C]),
        Err(ContentsReadError::TooShort { .. })
    ));
}

#[test]
fn revision_value_keeps_exact_source_provenance() {
    let bytes = header(CONTENTS_0X2C_MAGIC, 0x001A);
    let parsed = parse_preamble(StreamPath("/Synthetic".into()), &bytes)
        .expect("synthetic preamble must parse");

    assert_eq!(parsed.family, ContentsFamily::Family0x2c);
    assert_eq!(parsed.serialization_revision, 0x001A);
    assert_eq!(parsed.serialization_revision_source.offset, 12);
    assert_eq!(parsed.serialization_revision_source.len, 2);
}

#[test]
fn same_revision_scalar_can_exist_under_different_family_envelopes() {
    let revision = 0x001A;
    let old = parse_preamble(
        StreamPath("/SyntheticOld".into()),
        &header(CONTENTS_0X22_MAGIC, revision),
    )
    .expect("old family synthetic header must parse");
    let mature = parse_preamble(
        StreamPath("/SyntheticMature".into()),
        &header(CONTENTS_0X2C_MAGIC, revision),
    )
    .expect("mature family synthetic header must parse");

    assert_ne!(old.family, mature.family);
    assert_eq!(old.serialization_revision, mature.serialization_revision);
}

#[test]
fn family_and_revision_are_structural_coordinates_not_marketing_version() {
    let samples = [
        (CONTENTS_0X22_MAGIC, 0x02CD, ContentsFamily::Family0x22),
        (CONTENTS_0X2C_MAGIC, 0x0018, ContentsFamily::Family0x2c),
        (CONTENTS_0X2C_MAGIC, 0x001A, ContentsFamily::Family0x2c),
    ];

    for (magic, revision, expected_family) in samples {
        let parsed = parse_preamble(
            StreamPath("/Synthetic".into()),
            &header(magic, revision),
        )
        .expect("synthetic bounded classifier sample must parse");
        assert_eq!(parsed.family, expected_family);
        assert_eq!(parsed.serialization_revision, revision);
    }

    // Intentionally no mapping from these structural coordinates to Publisher 2000/2002/etc.
}
