use pub_core::StreamPath;
use pub_quill::{
    QuillToknTargetRecord, TOKN_PROPERTY_STATE, parse_confirmed_story_catalog,
};

fn decode_base64(input: &str) -> Vec<u8> {
    let mut output = Vec::with_capacity(input.len() * 3 / 4);
    let mut buffer = 0_u32;
    let mut bits = 0_u8;

    for byte in input.bytes() {
        let value = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            b'=' => break,
            byte if byte.is_ascii_whitespace() => continue,
            other => panic!("unexpected base64 byte: {other:#04x}"),
        };
        buffer = (buffer << 6) | u32::from(value);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            output.push((buffer >> bits) as u8);
            buffer &= if bits == 0 { 0 } else { (1_u32 << bits) - 1 };
        }
    }
    output
}

fn parse_fixture(encoded: &str) -> pub_quill::QuillStoryCatalog {
    let pub_bytes = decode_base64(encoded);
    let quill = pub_cfb::read_stream_reader(
        std::io::Cursor::new(pub_bytes.as_slice()),
        "/Quill/QuillSub/CONTENTS",
    )
    .expect("read pinned Publisher Quill stream");
    parse_confirmed_story_catalog(
        StreamPath("/Quill/QuillSub/CONTENTS".into()),
        &quill,
    )
    .expect("parse pinned Publisher Quill Story/TOKN catalog")
}

#[test]
fn apache_60685_generic_n3_tokn_uses_story_join_without_hyperlink_assumption() {
    let parsed = parse_fixture(include_str!("fixtures/60685.pub.b64"));
    let tokn = parsed
        .tokn
        .iter()
        .find(|tokn| {
            tokn.plc_count.value == 3
                && tokn
                    .effective_tokens
                    .first()
                    .is_some_and(|token| token.state_raw == Some(0x0600))
        })
        .expect("60685 compact generic N=3 TOKN");

    assert_eq!(tokn.story_ordinal.value, 0);
    assert_eq!(tokn.story_syid.value, parsed.syid.ids[0].value);
    assert_eq!(tokn.source.len, 88);
    assert_eq!(
        tokn.effective_tokens
            .iter()
            .map(|token| token.text_length_utf16)
            .collect::<Vec<_>>(),
        vec![Some(9), Some(15), Some(10)]
    );
    assert_eq!(
        tokn.effective_tokens
            .iter()
            .map(|token| token.kind_i32())
            .collect::<Vec<_>>(),
        vec![Some(7), Some(7), Some(7)]
    );
    assert!(tokn.target_section.is_none());
    assert!(tokn.opaque_tail.is_empty());
}

#[test]
fn apache_linkat10_target_bearing_tokn_uses_same_generic_reader() {
    let parsed = parse_fixture(include_str!("fixtures/LinkAt10.pub.b64"));
    let tokn = parsed
        .tokn
        .iter()
        .find(|tokn| {
            tokn.effective_tokens.first().is_some_and(|token| {
                token.state_raw == Some(0x08c0)
                    && token.text_length_utf16 == Some(4)
                    && token.kind_i32() == Some(1)
                    && token.attached_target_index == Some(0)
            })
        })
        .expect("LinkAt10 controlled URL TOKN");

    let ordinal = usize::from(tokn.story_ordinal.value);
    assert_eq!(tokn.story_syid.value, parsed.syid.ids[ordinal].value);
    let target = tokn.target_section.as_ref().expect("URL target section");
    assert_eq!(target.records.len(), 1);
    assert!(matches!(
        &target.records[0],
        QuillToknTargetRecord::Utf16String { text, .. }
            if text == "http://poi.apache.org/"
    ));

    // First-phase state remains raw wire state, not a named hyperlink mode.
    assert_eq!(
        tokn.first_phase[0]
            .property(TOKN_PROPERTY_STATE)
            .map(|value| value.value),
        Some(0x08c0)
    );
}
