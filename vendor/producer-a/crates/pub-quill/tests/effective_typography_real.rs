use pub_core::StreamPath;
use pub_quill::{
    QuillParagraphSelectorSource, QuillTypographyValueSource, parse_bounded_typography,
    parse_confirmed_story_catalog,
};
use std::io::Cursor;
use std::path::Path;

fn effective_counts(path: &Path) -> (usize, usize, usize, usize, Vec<u8>) {
    let pub_bytes = std::fs::read(path).expect("read pinned PUB fixture");
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
        parse_bounded_typography(&quill, &stories).expect("parse bounded effective typography");

    eprintln!(
        "{} typography fences: fdpc_unknown={:?} inheritance_unknown={:?} inheritance_reason={:?}",
        path.display(),
        typography.unknown_block_types_assumed_zero_length,
        typography.inheritance_unknown_block_types_assumed_zero_length,
        typography.effective_inheritance_unavailable_reason,
    );
    assert!(
        typography.effective_inheritance_unavailable_reason.is_none(),
        "effective inheritance must be available: {:?}",
        typography.effective_inheritance_unavailable_reason
    );
    assert!(
        typography
            .inheritance_unknown_block_types_assumed_zero_length
            .is_empty(),
        "real acceptance must not promote through unknown inheritance block widths"
    );

    let explicit_complete = typography
        .effective_runs
        .iter()
        .filter(|run| {
            run.font_source == QuillTypographyValueSource::ExplicitFdpc
                && run.text_size_source == QuillTypographyValueSource::ExplicitFdpc
        })
        .count();
    let inherited_explicit_selector = typography
        .effective_runs
        .iter()
        .filter(|run| {
            run.uses_inheritance()
                && run.inherited_selector_source
                    == Some(QuillParagraphSelectorSource::ExplicitFdpp0x19)
        })
        .count();
    let inherited_implicit_zero = typography
        .effective_runs
        .iter()
        .filter(|run| {
            run.uses_inheritance()
                && run.inherited_selector_source
                    == Some(
                        QuillParagraphSelectorSource::ImplicitStyleZeroFromBoundedEvidence,
                    )
        })
        .count();

    (
        typography.effective_runs.len(),
        explicit_complete,
        inherited_explicit_selector,
        inherited_implicit_zero,
        typography.unknown_block_types_assumed_zero_length,
    )
}

#[test]
#[ignore = "requires pinned Apache POI SampleNewsletter and SampleBrochure paths"]
fn real_pub_effective_typography_matches_product_authority_and_brochure_fence() {
    let newsletter = std::env::var_os("CHAPTERA_SAMPLE_NEWSLETTER")
        .map(std::path::PathBuf::from)
        .expect("CHAPTERA_SAMPLE_NEWSLETTER");
    let brochure = std::env::var_os("CHAPTERA_SAMPLE_BROCHURE")
        .map(std::path::PathBuf::from)
        .expect("CHAPTERA_SAMPLE_BROCHURE");

    let newsletter_counts = effective_counts(&newsletter);
    let brochure_counts = effective_counts(&brochure);

    eprintln!(
        "SampleNewsletter effective={} explicit={} inherited_explicit_selector={} inherited_implicit_zero={}",
        newsletter_counts.0,
        newsletter_counts.1,
        newsletter_counts.2,
        newsletter_counts.3,
    );
    eprintln!(
        "SampleNewsletter FDPC unknown fixed block types: {:?}",
        newsletter_counts.4
    );
    eprintln!(
        "SampleBrochure effective={} explicit={} inherited_explicit_selector={} inherited_implicit_zero={}",
        brochure_counts.0,
        brochure_counts.1,
        brochure_counts.2,
        brochure_counts.3,
    );
    eprintln!(
        "SampleBrochure FDPC unknown fixed block types: {:?}",
        brochure_counts.4
    );

    assert_eq!(newsletter_counts, (106, 18, 88, 0, Vec::new()));
    assert_eq!(brochure_counts, (0, 0, 0, 0, vec![0x02]));
}
