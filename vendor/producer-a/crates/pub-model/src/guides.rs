use crate::{PageId, SourceRef};
use serde::{Deserialize, Serialize};

/// Разные Publisher guide mechanisms не объединяются в один тип по внешнему виду.
///
/// Controlled runtime evidence различает publication-wide LayoutGuides и
/// page-local RulerGuides. Это разные semantic roles и разные persistence
/// projections.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PublisherGuideRole {
    PublicationLayoutGuides,
    PageRulerGuide,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RulerGuideAxis {
    Horizontal,
    Vertical,
}

/// Семантическая часть page-local ruler guide.
///
/// Тип координаты намеренно параметризован. `pub-model` пока не должен
/// подменять отдельный CDM geometry gate собственным числовым форматом.
/// Позже сюда можно подставить доказанный canonical physical-length type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RulerGuide<Position> {
    pub axis: RulerGuideAxis,
    pub position: Position,
}

/// A guide may cross the authoring boundary only after the source adapter has
/// grounded its page ownership, semantic role, axis and physical position.
///
/// Raw source provenance stays attached here for auditability; the layout
/// projection deliberately strips carrier/byte-range details and retains only
/// the semantic PublisherGuideRole.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroundedRulerGuide<Position> {
    pub page_id: PageId,
    pub role: PublisherGuideRole,
    pub guide: RulerGuide<Position>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub source_refs: Vec<SourceRef>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn publication_layout_and_page_ruler_guides_remain_distinct_roles() {
        assert_ne!(
            PublisherGuideRole::PublicationLayoutGuides,
            PublisherGuideRole::PageRulerGuide
        );
    }

    #[test]
    fn ruler_guide_does_not_choose_geometry_primitive_for_the_caller() {
        let guide = RulerGuide {
            axis: RulerGuideAxis::Vertical,
            position: 914_400_i64,
        };

        assert_eq!(guide.axis, RulerGuideAxis::Vertical);
        assert_eq!(guide.position, 914_400);
    }
}
