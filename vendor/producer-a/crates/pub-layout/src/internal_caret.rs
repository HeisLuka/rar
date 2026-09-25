//! Authoritative internal-caret geometry for bounded horizontal LTR shaping.
//!
//! The source of truth is explicit OpenType GDEF LigCaretList data from the same
//! pinned font bytes used by the HarfRust shaping path. This module never divides
//! glyph advance heuristically. Logical/grapheme eligibility is supplied by the
//! caller; scalar count alone never creates an internal caret.

use crate::shaping::{
    BOUNDED_SHAPER_REVISION, BoundedShapeError, BoundedShapedText, BoundedShapingRuntime,
    font_fingerprint_sha256, scale_font_units, shape_bounded_ltr_segment,
};
use pub_model::LengthEmu;
use read_fonts::{
    FontRef, TableProvider,
    tables::gdef::CaretValue,
    types::GlyphId,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

pub const INTERNAL_CARET_AUTHORITY_REVISION: &str =
    "chaptera.internal-caret-authority.gdef-format1.v1";
pub const GDEF_FORMAT1_AUTHORITY_SOURCE: &str =
    "opentype_gdef_ligature_caret_format1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InternalCaretStopAuthorityV1 {
    pub scalar_boundary: u32,
    pub cluster_start_scalar: u32,
    pub cluster_end_scalar: u32,
    pub glyph_id: u32,
    pub glyph_origin_x_emu: LengthEmu,
    pub gdef_coordinate_font_units: i16,
    pub caret_x_emu: LengthEmu,
    pub authority_source: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnsupportedInternalCaretV1 {
    pub scalar_boundary: u32,
    pub cluster_start_scalar: u32,
    pub cluster_end_scalar: u32,
    pub glyph_id: Option<u32>,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InternalCaretAuthorityResultV1 {
    pub protocol_version: String,
    pub font_fingerprint_sha256: String,
    pub face_index: u32,
    pub font_size_emu: LengthEmu,
    pub shaper_revision: String,
    pub scalar_base: u32,
    pub text_scalar_len: u32,
    pub eligible_internal_boundaries: Vec<u32>,
    pub shaped: BoundedShapedText,
    pub stops: Vec<InternalCaretStopAuthorityV1>,
    pub unsupported: Vec<UnsupportedInternalCaretV1>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InternalCaretAuthorityError {
    Shape(BoundedShapeError),
    ScalarIndexOverflow,
    DuplicateEligibleBoundary { scalar_boundary: u32 },
    EligibleBoundaryOutsideRun { scalar_boundary: u32 },
    NonMonotonicClusterStream,
    MetricScaleOverflow,
}

impl From<BoundedShapeError> for InternalCaretAuthorityError {
    fn from(value: BoundedShapeError) -> Self {
        Self::Shape(value)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum GdefCaretLookup {
    ExactFormat1(Vec<i16>),
    AbsentOrUnreadable,
    UnsupportedFormat,
}

fn lookup_gdef_ligature_carets_format1(
    font_bytes: &[u8],
    face_index: u32,
    glyph_id: u32,
) -> GdefCaretLookup {
    let Ok(font) = FontRef::from_index(font_bytes, face_index) else {
        return GdefCaretLookup::AbsentOrUnreadable;
    };
    let Ok(gdef) = font.gdef() else {
        return GdefCaretLookup::AbsentOrUnreadable;
    };
    let Some(ligature_list) = gdef.lig_caret_list() else {
        return GdefCaretLookup::AbsentOrUnreadable;
    };
    let Ok(ligature_list) = ligature_list else {
        return GdefCaretLookup::AbsentOrUnreadable;
    };
    let Ok(coverage) = ligature_list.coverage() else {
        return GdefCaretLookup::AbsentOrUnreadable;
    };
    let Some(coverage_index) = coverage.get(GlyphId::new(glyph_id)) else {
        return GdefCaretLookup::AbsentOrUnreadable;
    };
    let Ok(ligature_glyph) = ligature_list.lig_glyphs().get(usize::from(coverage_index)) else {
        return GdefCaretLookup::AbsentOrUnreadable;
    };

    let mut coordinates = Vec::with_capacity(usize::from(ligature_glyph.caret_count()));
    for offset in ligature_glyph.caret_value_offsets() {
        let resolved: Result<CaretValue<'_>, _> =
            offset.get().resolve(ligature_glyph.offset_data());
        let Ok(caret) = resolved else {
            return GdefCaretLookup::AbsentOrUnreadable;
        };
        match caret {
            CaretValue::Format1(value) => coordinates.push(value.coordinate()),
            CaretValue::Format2(_) | CaretValue::Format3(_) => {
                return GdefCaretLookup::UnsupportedFormat;
            }
        }
    }

    GdefCaretLookup::ExactFormat1(coordinates)
}

#[derive(Debug, Clone)]
struct ShapedCluster {
    start_scalar: u32,
    end_scalar: u32,
    glyph_indices: Vec<usize>,
}

fn shaped_clusters(
    shaped: &BoundedShapedText,
    run_end_scalar: u32,
) -> Result<Vec<ShapedCluster>, InternalCaretAuthorityError> {
    if shaped.glyphs.is_empty() {
        return Ok(Vec::new());
    }

    let mut clusters = Vec::<ShapedCluster>::new();
    let mut previous_cluster = None;

    for (index, glyph) in shaped.glyphs.iter().enumerate() {
        if let Some(previous) = previous_cluster
            && glyph.cluster < previous
        {
            return Err(InternalCaretAuthorityError::NonMonotonicClusterStream);
        }
        previous_cluster = Some(glyph.cluster);

        if let Some(last) = clusters.last_mut()
            && last.start_scalar == glyph.cluster
        {
            last.glyph_indices.push(index);
            continue;
        }

        if let Some(last) = clusters.last_mut() {
            last.end_scalar = glyph.cluster;
        }
        clusters.push(ShapedCluster {
            start_scalar: glyph.cluster,
            end_scalar: run_end_scalar,
            glyph_indices: vec![index],
        });
    }

    if let Some(last) = clusters.last_mut() {
        last.end_scalar = run_end_scalar;
    }
    Ok(clusters)
}

fn glyph_origins_x_emu(
    shaped: &BoundedShapedText,
) -> Result<Vec<LengthEmu>, InternalCaretAuthorityError> {
    let mut pen_x = 0i64;
    let mut origins = Vec::with_capacity(shaped.glyphs.len());
    for glyph in &shaped.glyphs {
        let origin = pen_x
            .checked_add(glyph.x_offset.get())
            .ok_or(InternalCaretAuthorityError::MetricScaleOverflow)?;
        origins.push(LengthEmu::new(origin));
        pen_x = pen_x
            .checked_add(glyph.x_advance.get())
            .ok_or(InternalCaretAuthorityError::MetricScaleOverflow)?;
    }
    Ok(origins)
}

fn unsupported_for_boundaries(
    out: &mut Vec<UnsupportedInternalCaretV1>,
    boundaries: &[u32],
    cluster: &ShapedCluster,
    glyph_id: Option<u32>,
    reason: &str,
) {
    out.extend(
        boundaries
            .iter()
            .copied()
            .map(|scalar_boundary| UnsupportedInternalCaretV1 {
                scalar_boundary,
                cluster_start_scalar: cluster.start_scalar,
                cluster_end_scalar: cluster.end_scalar,
                glyph_id,
                reason: reason.to_owned(),
            }),
    );
}

/// Resolve caller-approved logical boundaries that fall inside shaped clusters.
///
/// eligible_internal_boundaries is owned by the logical interaction/grapheme
/// policy. This function only determines physical geometry. Boundaries that are
/// cluster edges need no internal authority and are ignored here.
///
/// V1 admits only one-glyph horizontal-LTR clusters whose GDEF LigCaretList
/// entry contains CaretValue Format 1 coordinates. The number of caller-approved
/// internal boundaries must exactly match the number of GDEF ligature component
/// carets. Anything else is explicit unsupported.
pub fn resolve_internal_carets_ltr(
    text: &str,
    scalar_base: u32,
    runtime: &BoundedShapingRuntime<'_>,
    eligible_internal_boundaries: &[u32],
) -> Result<InternalCaretAuthorityResultV1, InternalCaretAuthorityError> {
    let text_scalar_len = u32::try_from(text.chars().count())
        .map_err(|_| InternalCaretAuthorityError::ScalarIndexOverflow)?;
    let run_end_scalar = scalar_base
        .checked_add(text_scalar_len)
        .ok_or(InternalCaretAuthorityError::ScalarIndexOverflow)?;

    let mut eligible = eligible_internal_boundaries.to_vec();
    eligible.sort_unstable();
    let mut seen = BTreeSet::new();
    for boundary in &eligible {
        if !seen.insert(*boundary) {
            return Err(InternalCaretAuthorityError::DuplicateEligibleBoundary {
                scalar_boundary: *boundary,
            });
        }
        if *boundary <= scalar_base || *boundary >= run_end_scalar {
            return Err(InternalCaretAuthorityError::EligibleBoundaryOutsideRun {
                scalar_boundary: *boundary,
            });
        }
    }

    let shaped = shape_bounded_ltr_segment(text, scalar_base, runtime)?;
    let clusters = shaped_clusters(&shaped, run_end_scalar)?;
    let origins = glyph_origins_x_emu(&shaped)?;

    let mut stops = Vec::new();
    let mut unsupported = Vec::new();

    for cluster in &clusters {
        let boundaries = eligible
            .iter()
            .copied()
            .filter(|boundary| {
                cluster.start_scalar < *boundary && *boundary < cluster.end_scalar
            })
            .collect::<Vec<_>>();
        if boundaries.is_empty() {
            continue;
        }

        if cluster.glyph_indices.len() != 1 {
            unsupported_for_boundaries(
                &mut unsupported,
                &boundaries,
                cluster,
                None,
                "multi_glyph_cluster_has_no_single_ligature_gdef_authority",
            );
            continue;
        }

        let glyph_index = cluster.glyph_indices[0];
        let glyph = &shaped.glyphs[glyph_index];
        let coordinates = match lookup_gdef_ligature_carets_format1(
            runtime.font_bytes,
            runtime.face_index,
            glyph.glyph_id,
        ) {
            GdefCaretLookup::ExactFormat1(values) => values,
            GdefCaretLookup::AbsentOrUnreadable => {
                unsupported_for_boundaries(
                    &mut unsupported,
                    &boundaries,
                    cluster,
                    Some(glyph.glyph_id),
                    "gdef_ligature_caret_absent_or_unreadable",
                );
                continue;
            }
            GdefCaretLookup::UnsupportedFormat => {
                unsupported_for_boundaries(
                    &mut unsupported,
                    &boundaries,
                    cluster,
                    Some(glyph.glyph_id),
                    "gdef_ligature_caret_format_not_admitted_v1",
                );
                continue;
            }
        };

        if coordinates.len() != boundaries.len() {
            unsupported_for_boundaries(
                &mut unsupported,
                &boundaries,
                cluster,
                Some(glyph.glyph_id),
                "gdef_component_caret_count_does_not_match_eligible_boundaries",
            );
            continue;
        }
        if coordinates.windows(2).any(|pair| pair[0] >= pair[1]) {
            unsupported_for_boundaries(
                &mut unsupported,
                &boundaries,
                cluster,
                Some(glyph.glyph_id),
                "gdef_ligature_carets_not_strictly_increasing",
            );
            continue;
        }

        let origin_x = origins[glyph_index];
        for (scalar_boundary, coordinate) in boundaries.into_iter().zip(coordinates) {
            let scaled = scale_font_units(
                i32::from(coordinate),
                runtime.font_size_emu,
                shaped.units_per_em,
            )?;
            let caret_x = origin_x
                .get()
                .checked_add(scaled.get())
                .ok_or(InternalCaretAuthorityError::MetricScaleOverflow)?;
            stops.push(InternalCaretStopAuthorityV1 {
                scalar_boundary,
                cluster_start_scalar: cluster.start_scalar,
                cluster_end_scalar: cluster.end_scalar,
                glyph_id: glyph.glyph_id,
                glyph_origin_x_emu: origin_x,
                gdef_coordinate_font_units: coordinate,
                caret_x_emu: LengthEmu::new(caret_x),
                authority_source: GDEF_FORMAT1_AUTHORITY_SOURCE.to_owned(),
            });
        }
    }

    stops.sort_by_key(|stop| stop.scalar_boundary);
    unsupported.sort_by_key(|item| item.scalar_boundary);

    Ok(InternalCaretAuthorityResultV1 {
        protocol_version: INTERNAL_CARET_AUTHORITY_REVISION.to_owned(),
        font_fingerprint_sha256: font_fingerprint_sha256(runtime.font_bytes),
        face_index: runtime.face_index,
        font_size_emu: runtime.font_size_emu,
        shaper_revision: BOUNDED_SHAPER_REVISION.to_owned(),
        scalar_base,
        text_scalar_len,
        eligible_internal_boundaries: eligible,
        shaped,
        stops,
        unsupported,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::BoundedLayoutEnvironment;
    use pub_model::EMU_PER_POINT;

    fn runtime(font_bytes: &[u8]) -> BoundedShapingRuntime<'_> {
        BoundedShapingRuntime {
            layout: BoundedLayoutEnvironment {
                engine_revision: "layout-text-internal-caret-auth-01".into(),
                font_set_fingerprint: font_fingerprint_sha256(font_bytes),
                resource_fingerprint: "resources:none".into(),
            },
            face_index: 0,
            font_size_emu: LengthEmu::new(12 * EMU_PER_POINT),
            font_bytes,
        }
    }

    #[test]
    fn real_fi_gsub_uses_explicit_gdef_coordinate_not_advance_division() {
        let font = font_test_data::NOTOSERIF_AUTOHINT_SHAPING;
        let result =
            resolve_internal_carets_ltr("fi", 0, &runtime(font), &[1]).expect("GDEF authority");
        assert_eq!(1, result.stops.len());
        assert!(result.unsupported.is_empty());

        let stop = &result.stops[0];
        assert_eq!(1, stop.scalar_boundary);
        assert_eq!(7, stop.glyph_id, "fixture GSUB fi must lower to glyph00007");
        assert_eq!(332, stop.gdef_coordinate_font_units);
        assert_eq!(LengthEmu::new(50_597), stop.caret_x_emu);
        assert_eq!(GDEF_FORMAT1_AUTHORITY_SOURCE, stop.authority_source);
        assert_eq!(1, result.shaped.glyphs.len());
        assert_eq!(0, result.shaped.glyphs[0].cluster);

        let heuristic_half = result.shaped.glyphs[0].x_advance.get() / 2;
        assert_ne!(
            heuristic_half,
            stop.caret_x_emu.get(),
            "proof must be distinguishable from advance/2"
        );
    }

    #[test]
    fn scalar_count_alone_never_creates_combining_or_zwj_internal_stops() {
        let font = font_test_data::NOTOSERIF_AUTOHINT_SHAPING;
        let combining =
            resolve_internal_carets_ltr("f\u{0301}", 0, &runtime(font), &[]).expect("shape");
        let zwj =
            resolve_internal_carets_ltr("f\u{200d}i", 0, &runtime(font), &[]).expect("shape");
        assert!(combining.stops.is_empty());
        assert!(combining.unsupported.is_empty());
        assert!(zwj.stops.is_empty());
        assert!(zwj.unsupported.is_empty());
    }

    #[test]
    fn font_glyph_without_ligature_caret_authority_is_explicitly_absent() {
        let font = font_test_data::NOTOSERIF_AUTOHINT_SHAPING;
        assert_eq!(
            GdefCaretLookup::AbsentOrUnreadable,
            lookup_gdef_ligature_carets_format1(font, 0, 2),
            "plain f glyph is not covered by the fixture LigCaretList"
        );
    }

    #[test]
    fn ffi_maps_two_policy_approved_boundaries_to_two_explicit_gdef_carets() {
        let font = font_test_data::NOTOSERIF_AUTOHINT_SHAPING;
        let result =
            resolve_internal_carets_ltr("ffi", 9, &runtime(font), &[10, 11]).expect("GDEF");
        assert_eq!(2, result.stops.len());
        assert_eq!([10, 11], [result.stops[0].scalar_boundary, result.stops[1].scalar_boundary]);
        assert_eq!([334, 668], [
            result.stops[0].gdef_coordinate_font_units,
            result.stops[1].gdef_coordinate_font_units,
        ]);
        assert!(result.unsupported.is_empty());
    }

    #[test]
    fn caller_policy_boundary_count_mismatch_fails_closed_not_interpolated() {
        let font = font_test_data::NOTOSERIF_AUTOHINT_SHAPING;
        let result =
            resolve_internal_carets_ltr("ffi", 0, &runtime(font), &[1]).expect("bounded result");
        assert!(result.stops.is_empty());
        assert_eq!(1, result.unsupported.len());
        assert_eq!(
            "gdef_component_caret_count_does_not_match_eligible_boundaries",
            result.unsupported[0].reason
        );
    }

    #[test]
    fn duplicate_or_out_of_run_policy_boundaries_are_invalid_input() {
        let font = font_test_data::NOTOSERIF_AUTOHINT_SHAPING;
        assert_eq!(
            Err(InternalCaretAuthorityError::DuplicateEligibleBoundary {
                scalar_boundary: 1
            }),
            resolve_internal_carets_ltr("fi", 0, &runtime(font), &[1, 1])
        );
        assert_eq!(
            Err(InternalCaretAuthorityError::EligibleBoundaryOutsideRun {
                scalar_boundary: 2
            }),
            resolve_internal_carets_ltr("fi", 0, &runtime(font), &[2])
        );
    }

    #[test]
    fn same_pinned_input_is_byte_identical_json() {
        let font = font_test_data::NOTOSERIF_AUTOHINT_SHAPING;
        let runtime = runtime(font);
        let a = resolve_internal_carets_ltr("fi", 0, &runtime, &[1]).expect("a");
        let b = resolve_internal_carets_ltr("fi", 0, &runtime, &[1]).expect("b");
        assert_eq!(a, b);
        assert_eq!(
            serde_json::to_vec(&a).expect("serialize"),
            serde_json::to_vec(&b).expect("serialize")
        );
    }
}
