use pub_model::{
    AuthorityClass, GroundedRulerGuide, LengthEmu, Page, PageId, PublisherGuideRole,
    ReadConfidence, RulerGuide, RulerGuideAxis, SourceDescriptor, SourceRef, SourceRole,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum PubGuideObservation {
    Grounded {
        page_id: PageId,
        role: PublisherGuideRole,
        axis: RulerGuideAxis,
        position_emu: i64,
        source_refs: Vec<SourceRef>,
    },
    Absent {
        page_id: PageId,
        role: PublisherGuideRole,
    },
    Ambiguous {
        page_id: PageId,
        role: PublisherGuideRole,
        reason: String,
        source_refs: Vec<SourceRef>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "code", rename_all = "snake_case")]
pub enum PubGuideProjectionDiagnostic {
    UnknownPage {
        page_id: PageId,
    },
    OutOfPageRange {
        page_id: PageId,
        axis: RulerGuideAxis,
        position_emu: i64,
    },
    NonAuthoritativeProvenance {
        page_id: PageId,
        role: PublisherGuideRole,
    },
    AmbiguousSource {
        page_id: PageId,
        role: PublisherGuideRole,
        reason: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct PubGroundedGuideBuild {
    pub guides: Vec<GroundedRulerGuide<LengthEmu>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<PubGuideProjectionDiagnostic>,
}

pub fn materialize_grounded_guides(
    source: &SourceDescriptor,
    pages: &BTreeMap<PageId, Page>,
    observations: impl IntoIterator<Item = PubGuideObservation>,
) -> PubGroundedGuideBuild {
    let mut build = PubGroundedGuideBuild::default();

    for observation in observations {
        match observation {
            PubGuideObservation::Absent { .. } => {}
            PubGuideObservation::Ambiguous {
                page_id,
                role,
                reason,
                ..
            } => {
                build
                    .diagnostics
                    .push(PubGuideProjectionDiagnostic::AmbiguousSource {
                        page_id,
                        role,
                        reason,
                    });
            }
            PubGuideObservation::Grounded {
                page_id,
                role,
                axis,
                position_emu,
                source_refs,
            } => {
                let Some(page) = pages.get(&page_id) else {
                    build
                        .diagnostics
                        .push(PubGuideProjectionDiagnostic::UnknownPage { page_id });
                    continue;
                };

                let limit = match axis {
                    RulerGuideAxis::Horizontal => page.size.height.get(),
                    RulerGuideAxis::Vertical => page.size.width.get(),
                };
                if position_emu < 0 || position_emu > limit {
                    build
                        .diagnostics
                        .push(PubGuideProjectionDiagnostic::OutOfPageRange {
                            page_id,
                            axis,
                            position_emu,
                        });
                    continue;
                }

                let authoritative = !source_refs.is_empty()
                    && source_refs.iter().all(|source_ref| {
                        source_ref.validate_primary_source(source).is_ok()
                            && source_ref.role == SourceRole::Projection
                            && source_ref.authority == AuthorityClass::Authoritative
                            && source_ref.confidence == Some(ReadConfidence::Exact)
                    });
                if !authoritative {
                    build.diagnostics.push(
                        PubGuideProjectionDiagnostic::NonAuthoritativeProvenance { page_id, role },
                    );
                    continue;
                }

                build.guides.push(GroundedRulerGuide {
                    page_id,
                    role,
                    guide: RulerGuide {
                        axis,
                        position: LengthEmu::new(position_emu),
                    },
                    source_refs,
                });
            }
        }
    }

    build.guides.sort_by(|left, right| {
        (
            left.page_id,
            role_order(left.role),
            axis_order(left.guide.axis),
            left.guide.position,
        )
            .cmp(&(
                right.page_id,
                role_order(right.role),
                axis_order(right.guide.axis),
                right.guide.position,
            ))
    });
    build
}

fn role_order(role: PublisherGuideRole) -> u8 {
    match role {
        PublisherGuideRole::PublicationLayoutGuides => 0,
        PublisherGuideRole::PageRulerGuide => 1,
    }
}

fn axis_order(axis: RulerGuideAxis) -> u8 {
    match axis {
        RulerGuideAxis::Horizontal => 0,
        RulerGuideAxis::Vertical => 1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pub_model::{
        CanonicalId, DocumentId, EMU_PER_INCH, PageId, Sha256Digest, Size2D, SourceDescriptor,
    };

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
}
