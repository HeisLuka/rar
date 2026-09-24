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
