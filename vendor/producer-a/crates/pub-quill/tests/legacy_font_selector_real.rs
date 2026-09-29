use pub_core::StreamPath;
use pub_quill::{parse_bounded_typography, parse_confirmed_story_catalog};
use std::io::Cursor;
use std::path::PathBuf;

const TARGET_TEXT: &str = "Caladea, ";
const RAW_SELECTOR_OFFSET: usize = 0x888;
const RAW_SELECTOR_BYTES: [u8; 18] = [
    0x24, 0x8a, 0x10, 0x00, 0x00, 0x00,
    0x00, 0x18, 0x01, 0x00,
    0x08, 0x18, 0x01, 0x00,
    0x10, 0x18, 0x01, 0x00,
];
const STYLE_SELECTOR_PAYLOAD: [u8; 12] = [
    0x00, 0x18, 0x01, 0x00,
    0x08, 0x18, 0x01, 0x00,
    0x10, 0x18, 0x01, 0x00,
];

fn utf16le_bytes(text: &str) -> Vec<u8> {
    text.encode_utf16()
        .flat_map(u16::to_le_bytes)
        .collect::<Vec<_>>()
}

#[test]
#[ignore = "requires CHAPTERA_FONTSEL_022 exact fosnola/libmspub-test fixture"]
fn publisher2000_fixed_0x24_selector_resolves_caladea() {
    let path = std::env::var_os("CHAPTERA_FONTSEL_022")
        .map(PathBuf::from)
        .expect("CHAPTERA_FONTSEL_022");
    let pub_bytes = std::fs::read(&path).expect("read exact Publisher2000 fixture");
    assert_eq!(pub_bytes.len(), 21_504);
    assert_eq!(&pub_bytes[..8], &hex_cfb_magic());

    let quill = pub_cfb::read_stream_reader(
        Cursor::new(pub_bytes.as_slice()),
        "/Quill/QuillSub/CONTENTS",
    )
    .expect("read Quill stream");
    let stories = parse_confirmed_story_catalog(
        StreamPath("/Quill/QuillSub/CONTENTS".into()),
        &quill,
    )
    .expect("parse Story catalog");
    let typography =
        parse_bounded_typography(&quill, &stories).expect("parse bounded typography");

    assert_eq!(typography.font_names.get(0).map(String::as_str), Some("Times New Roman"));
    assert_eq!(typography.font_names.get(1).map(String::as_str), Some("Caladea"));

    let needle = utf16le_bytes(TARGET_TEXT);
    let needle_units = u32::try_from(TARGET_TEXT.encode_utf16().count()).expect("needle units");
    let mut global_story_start = 0_u32;
    let mut matches = Vec::new();

    for story in &stories.stories {
        for byte_start in 0..=story.utf16le.len().saturating_sub(needle.len()) {
            if byte_start % 2 == 0
                && story.utf16le[byte_start..byte_start + needle.len()] == needle
            {
                let story_start_utf16 = u32::try_from(byte_start / 2).expect("story offset");
                matches.push((
                    global_story_start + story_start_utf16,
                    global_story_start + story_start_utf16 + needle_units,
                ));
            }
        }
        global_story_start = global_story_start
            .checked_add(story.utf16_code_units)
            .expect("global Story extent");
    }

    assert_eq!(matches.len(), 1, "target self-labelled span must be unique");
    let (target_start, target_end) = matches[0];
    let range = typography
        .ranges
        .iter()
        .find(|range| {
            range.global_start_utf16 <= target_start && range.global_end_utf16 >= target_end
        })
        .expect("FDPC range covering Caladea span");

    assert_eq!(range.font_indices, vec![1]);
    assert_eq!(range.font_names, vec!["Caladea".to_owned()]);

    assert_eq!(
        &quill[RAW_SELECTOR_OFFSET..RAW_SELECTOR_OFFSET + RAW_SELECTOR_BYTES.len()],
        &RAW_SELECTOR_BYTES,
        "registered raw Publisher2000 0x24 selector witness drifted"
    );

    let style_start = usize::try_from(range.fdpc_style_source.offset).expect("style start");
    let style_end = style_start
        .checked_add(usize::try_from(range.fdpc_style_source.len).expect("style len"))
        .expect("style end");
    let style = &quill[style_start..style_end];
    assert!(
        style
            .windows(STYLE_SELECTOR_PAYLOAD.len())
            .any(|window| window == STYLE_SELECTOR_PAYLOAD),
        "Caladea range must retain the exact three fixed type-0x18 selectors"
    );
}

fn hex_cfb_magic() -> [u8; 8] {
    [0xd0, 0xcf, 0x11, 0xe0, 0xa1, 0xb1, 0x1a, 0xe1]
}
