use crate::BoundedShapedGlyph;
use serde::{Deserialize, Serialize};
use std::fmt;
use uax14_linebreak::{BreakOpportunity, UNICODE_VERSION, linebreaks_iter};

pub const BOUNDED_BREAK_POLICY_REVISION: &str = "uax14-linebreak-0.1.0";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BoundedBreakKind {
    Allowed,
    Mandatory,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BoundedBreakCandidate {
    /// Zero-based Unicode scalar boundary where the next line would start.
    pub scalar_boundary: u32,
    pub kind: BoundedBreakKind,
    /// True when the already-shaped full run can be split at this boundary.
    pub safe_without_reshaping: bool,
    /// True when the break remains semantically valid but the two sides must be
    /// shaped independently before final placement.
    pub requires_reshaping: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BoundedBreakPolicy {
    pub policy_revision: String,
    pub unicode_version: (u8, u8, u8),
    pub candidates: Vec<BoundedBreakCandidate>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BoundedBreakPolicyError {
    ScalarIndexOverflow,
    GlyphClusterOutOfRange { cluster: u32, scalar_count: u32 },
}

impl fmt::Display for BoundedBreakPolicyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ScalarIndexOverflow => {
                write!(f, "text contains more Unicode scalars than u32 can index")
            }
            Self::GlyphClusterOutOfRange {
                cluster,
                scalar_count,
            } => write!(
                f,
                "glyph cluster {cluster} is outside scalar count {scalar_count}"
            ),
        }
    }
}

impl std::error::Error for BoundedBreakPolicyError {}

/// Builds the default Unicode 17 UAX #14 break policy and overlays shaping
/// safety from an already-shaped LTR run.
///
/// UAX #14 decides whether a boundary is Allowed or Mandatory. HarfRust's
/// cluster state decides only whether the existing full-run shape can be reused
/// across that boundary. Unsafe candidates are retained and marked as requiring
/// reshaping; they are never silently discarded.
pub fn break_policy_for_shaped_text(
    text: &str,
    glyphs: &[BoundedShapedGlyph],
) -> Result<BoundedBreakPolicy, BoundedBreakPolicyError> {
    let scalar_count_usize = text.chars().count();
    let scalar_count = u32::try_from(scalar_count_usize)
        .map_err(|_| BoundedBreakPolicyError::ScalarIndexOverflow)?;

    for glyph in glyphs {
        if glyph.cluster >= scalar_count && scalar_count != 0 {
            return Err(BoundedBreakPolicyError::GlyphClusterOutOfRange {
                cluster: glyph.cluster,
                scalar_count,
            });
        }
    }

    let candidates = linebreaks_iter(text.chars().enumerate(), scalar_count_usize)
        .map(|(boundary, opportunity)| {
            let scalar_boundary = u32::try_from(boundary)
                .map_err(|_| BoundedBreakPolicyError::ScalarIndexOverflow)?;
            let safe_without_reshaping = boundary_is_safe(glyphs, scalar_boundary, scalar_count);

            let kind = match opportunity {
                BreakOpportunity::Allowed => BoundedBreakKind::Allowed,
                BreakOpportunity::Mandatory => BoundedBreakKind::Mandatory,
            };

            Ok(BoundedBreakCandidate {
                scalar_boundary,
                kind,
                safe_without_reshaping,
                requires_reshaping: !safe_without_reshaping,
            })
        })
        .collect::<Result<Vec<_>, BoundedBreakPolicyError>>()?;

    Ok(BoundedBreakPolicy {
        policy_revision: BOUNDED_BREAK_POLICY_REVISION.into(),
        unicode_version: UNICODE_VERSION,
        candidates,
    })
}

fn boundary_is_safe(
    glyphs: &[BoundedShapedGlyph],
    scalar_boundary: u32,
    scalar_count: u32,
) -> bool {
    if scalar_boundary == 0 || scalar_boundary == scalar_count {
        return true;
    }

    let mut found = false;
    for glyph in glyphs
        .iter()
        .filter(|glyph| glyph.cluster == scalar_boundary)
    {
        found = true;
        if glyph.unsafe_to_break {
            return false;
        }
    }

    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use pub_model::LengthEmu;

    fn glyph(cluster: u32, unsafe_to_break: bool) -> BoundedShapedGlyph {
        BoundedShapedGlyph {
            glyph_id: cluster + 1,
            cluster,
            x_advance: LengthEmu::new(100),
            y_advance: LengthEmu::ZERO,
            x_offset: LengthEmu::ZERO,
            y_offset: LengthEmu::ZERO,
            unsafe_to_break,
        }
    }

    #[test]
    fn unicode17_space_and_end_are_scalar_indexed() {
        let text = "Hello world";
        let glyphs: Vec<_> = (0..11).map(|cluster| glyph(cluster, false)).collect();

        let policy = break_policy_for_shaped_text(text, &glyphs).unwrap();

        assert_eq!(policy.unicode_version, (17, 0, 0));
        assert_eq!(policy.policy_revision, BOUNDED_BREAK_POLICY_REVISION);
        assert_eq!(
            policy.candidates,
            vec![
                BoundedBreakCandidate {
                    scalar_boundary: 6,
                    kind: BoundedBreakKind::Allowed,
                    safe_without_reshaping: true,
                    requires_reshaping: false,
                },
                BoundedBreakCandidate {
                    scalar_boundary: 11,
                    kind: BoundedBreakKind::Mandatory,
                    safe_without_reshaping: true,
                    requires_reshaping: false,
                },
            ]
        );
    }

    #[test]
    fn mandatory_break_is_retained_when_existing_shape_is_unsafe() {
        let text = "a\nb";
        let glyphs = vec![glyph(0, false), glyph(1, false), glyph(2, true)];

        let policy = break_policy_for_shaped_text(text, &glyphs).unwrap();
        let hard_break = policy
            .candidates
            .iter()
            .find(|candidate| candidate.scalar_boundary == 2)
            .expect("newline must produce a mandatory break");

        assert_eq!(hard_break.kind, BoundedBreakKind::Mandatory);
        assert!(!hard_break.safe_without_reshaping);
        assert!(hard_break.requires_reshaping);
    }

    #[test]
    fn allowed_break_is_retained_when_reshaping_is_required() {
        let text = "a b";
        let glyphs = vec![glyph(0, false), glyph(1, false), glyph(2, true)];

        let policy = break_policy_for_shaped_text(text, &glyphs).unwrap();
        let word_break = policy
            .candidates
            .iter()
            .find(|candidate| candidate.scalar_boundary == 2)
            .expect("space must produce an allowed break");

        assert_eq!(word_break.kind, BoundedBreakKind::Allowed);
        assert!(word_break.requires_reshaping);
    }

    #[test]
    fn missing_cluster_boundary_requires_reshaping_instead_of_guessing() {
        let text = "a b";
        let glyphs = vec![glyph(0, false), glyph(1, false)];

        let policy = break_policy_for_shaped_text(text, &glyphs).unwrap();
        let word_break = policy
            .candidates
            .iter()
            .find(|candidate| candidate.scalar_boundary == 2)
            .unwrap();

        assert!(!word_break.safe_without_reshaping);
        assert!(word_break.requires_reshaping);
    }

    #[test]
    fn out_of_range_cluster_fails_closed() {
        let error = break_policy_for_shaped_text("ab", &[glyph(2, false)]).unwrap_err();

        assert_eq!(
            error,
            BoundedBreakPolicyError::GlyphClusterOutOfRange {
                cluster: 2,
                scalar_count: 2,
            }
        );
    }
}
