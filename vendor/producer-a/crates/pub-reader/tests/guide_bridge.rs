use pub_model::{
    AuthorityClass, CanonicalId, EMU_PER_INCH, LengthEmu, Page, PageId, PublisherGuideRole,
    ReadConfidence, RulerGuideAxis, Sha256Digest, Size2D, SourceDescriptor, SourceRef, SourceRole,
};
use pub_reader::{PubGuideObservation, PubGuideProjectionDiagnostic, materialize_grounded_guides};
use std::collections::BTreeMap;

fn page_id() -> PageId {
    PageId::from_canonical(CanonicalId::from_bytes([0x22; 16]))
}

fn source() -> SourceDescriptor {
    SourceDescriptor {
        format: "pub-mature-0x2c".into(),
        format_version: Some("0x2c".into()),
        adapter_version: "pub-rs".into(),
        source_hash: "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
            .parse::<Sha256Digest>()
            .unwrap(),
    }
}

fn page() -> Page {
    Page {
        id: page_id(),
        size: Size2D::new(
            LengthEmu::new(8 * EMU_PER_INCH),
            LengthEmu::new(11 * EMU_PER_INCH),
        ),
        bleed: None,
        margins: None,
        children: Vec::new(),
        extensions: Vec::new(),
    }
}

fn exact_ref() -> SourceRef {
    let source = source();
    SourceRef {
        format: source.format,
        adapter_version: source.adapter_version,
        source_hash: source.source_hash,
        carrier: "/Contents".into(),
        object_key: Some("contents/0x2c/seq/305".into()),
        path: Some("Margins/RulerGuide".into()),
        byte_range: None,
        role: SourceRole::Projection,
        authority: AuthorityClass::Authoritative,
        confidence: Some(ReadConfidence::Exact),
    }
}

#[test]
fn only_exact_grounded_observations_materialize() {
    let pages = BTreeMap::from([(page_id(), page())]);
    let build = materialize_grounded_guides(
        &source(),
        &pages,
        vec![
            PubGuideObservation::Grounded {
                page_id: page_id(),
                role: PublisherGuideRole::PageRulerGuide,
                axis: RulerGuideAxis::Vertical,
                position_emu: EMU_PER_INCH,
                source_refs: vec![exact_ref()],
            },
            PubGuideObservation::Absent {
                page_id: page_id(),
                role: PublisherGuideRole::PublicationLayoutGuides,
            },
            PubGuideObservation::Ambiguous {
                page_id: page_id(),
                role: PublisherGuideRole::PageRulerGuide,
                reason: "position/count grammar not grounded".into(),
                source_refs: vec![exact_ref()],
            },
        ],
    );

    assert_eq!(build.guides.len(), 1);
    assert_eq!(build.guides[0].role, PublisherGuideRole::PageRulerGuide);
    assert_eq!(build.guides[0].guide.position, LengthEmu::new(EMU_PER_INCH));
    assert!(build.diagnostics.iter().any(|diagnostic| matches!(
        diagnostic,
        PubGuideProjectionDiagnostic::AmbiguousSource { .. }
    )));
}

#[test]
fn ungrounded_or_out_of_range_state_never_synthesizes_a_guide() {
    let pages = BTreeMap::from([(page_id(), page())]);
    let mut weak_ref = exact_ref();
    weak_ref.confidence = Some(ReadConfidence::Inferred);

    let build = materialize_grounded_guides(
        &source(),
        &pages,
        vec![
            PubGuideObservation::Grounded {
                page_id: page_id(),
                role: PublisherGuideRole::PublicationLayoutGuides,
                axis: RulerGuideAxis::Horizontal,
                position_emu: 12 * EMU_PER_INCH,
                source_refs: vec![exact_ref()],
            },
            PubGuideObservation::Grounded {
                page_id: page_id(),
                role: PublisherGuideRole::PageRulerGuide,
                axis: RulerGuideAxis::Vertical,
                position_emu: EMU_PER_INCH,
                source_refs: vec![weak_ref],
            },
        ],
    );

    assert!(build.guides.is_empty());
    assert_eq!(build.diagnostics.len(), 2);
}
