use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const RASTER_CONTRACT_VERSION_V1: &str = "chaptera.render-raster-contract.v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AaClassV1 {
    None,
    AnalyticCoverage,
    Multisample4,
    BackendNativeBounded,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SamplingFilterV1 {
    Nearest,
    Linear,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PixelCenterConventionV1 {
    PixelEdgesAtIntegersCentersAtHalf,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ViewTransformV1 {
    pub emu_per_css_px: u64,
    pub zoom_ppm: u64,
    pub pan_x_milli_css_px: i64,
    pub pan_y_milli_css_px: i64,
    pub dpr_milli: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SurfaceTargetV1 {
    pub width_px: u32,
    pub height_px: u32,
    pub surface_generation: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RenderRasterContractV1 {
    pub contract_version: String,
    pub pixel_center: PixelCenterConventionV1,
    pub aa_class: AaClassV1,
    pub magnification_filter: SamplingFilterV1,
    pub minification_filter: SamplingFilterV1,
    pub mip_policy: String,
    pub stroke_policy: String,
    pub glyph_quality_class: String,
    pub path_quality_class: String,
    pub color_contract_id: String,
    pub view: ViewTransformV1,
    pub surface: SurfaceTargetV1,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackendRasterCapabilitiesV1 {
    pub backend_id: String,
    pub supported_aa: Vec<AaClassV1>,
    pub supported_filters: Vec<SamplingFilterV1>,
    pub supports_explicit_pixel_center_adapter: bool,
    pub supports_straight_alpha_sampling: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompatibilityV1 {
    pub compatible: bool,
    pub degraded: bool,
    pub reasons: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RasterCacheKeyV1 {
    pub material_identity: String,
    pub raster_contract_version: String,
    pub device_scale_bucket: String,
    pub aa_class: AaClassV1,
    pub magnification_filter: SamplingFilterV1,
    pub minification_filter: SamplingFilterV1,
    pub color_contract_id: String,
    pub backend_generation: u64,
    pub derivative_identity: String,
    pub mip_level: Option<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RasterReceiptV1 {
    pub contract_version: String,
    pub surface_generation: u64,
    pub dpr_milli: u32,
    pub surface_width_px: u32,
    pub surface_height_px: u32,
    pub aa_class: AaClassV1,
    pub magnification_filter: SamplingFilterV1,
    pub minification_filter: SamplingFilterV1,
    pub pixel_center: PixelCenterConventionV1,
    pub cache_key_digest: String,
    pub canonical_geometry_mutated: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DevicePointV1 {
    pub x: f64,
    pub y: f64,
}

pub fn baseline_contract_v1(
    view: ViewTransformV1,
    surface: SurfaceTargetV1,
    color_contract_id: impl Into<String>,
) -> Result<RenderRasterContractV1, String> {
    if view.emu_per_css_px == 0 || view.zoom_ppm == 0 || view.dpr_milli == 0 {
        return Err("view scale inputs must be positive".into());
    }
    if surface.width_px == 0 || surface.height_px == 0 {
        return Err("surface dimensions must be positive".into());
    }
    Ok(RenderRasterContractV1 {
        contract_version: RASTER_CONTRACT_VERSION_V1.into(),
        pixel_center: PixelCenterConventionV1::PixelEdgesAtIntegersCentersAtHalf,
        aa_class: AaClassV1::AnalyticCoverage,
        magnification_filter: SamplingFilterV1::Linear,
        minification_filter: SamplingFilterV1::Linear,
        mip_policy: "selected_material_mips_are_runtime_representation_not_derivative_identity"
            .into(),
        stroke_policy: "centerline_coverage_preserve_subpixel_width_no_hairline_inference".into(),
        glyph_quality_class: "settled_device_scale_bucket_v1".into(),
        path_quality_class: "settled_device_scale_tolerance_v1".into(),
        color_contract_id: color_contract_id.into(),
        view,
        surface,
    })
}

pub fn map_emu_point_to_device(
    x_emu: i64,
    y_emu: i64,
    contract: &RenderRasterContractV1,
) -> DevicePointV1 {
    let v = &contract.view;
    let css_scale = (v.zoom_ppm as f64 / 1_000_000.0) / v.emu_per_css_px as f64;
    let dpr = v.dpr_milli as f64 / 1_000.0;
    DevicePointV1 {
        x: (x_emu as f64 * css_scale + v.pan_x_milli_css_px as f64 / 1_000.0) * dpr,
        y: (y_emu as f64 * css_scale + v.pan_y_milli_css_px as f64 / 1_000.0) * dpr,
    }
}

pub fn normalized_to_integer_center_backend(point: DevicePointV1) -> DevicePointV1 {
    DevicePointV1 {
        x: point.x - 0.5,
        y: point.y - 0.5,
    }
}

pub fn validate_pixel_center_adapter(
    normalized: DevicePointV1,
    backend_native: DevicePointV1,
    backend_integer_coordinates_are_centers: bool,
) -> Result<(), String> {
    let expected = if backend_integer_coordinates_are_centers {
        normalized_to_integer_center_backend(normalized)
    } else {
        normalized
    };
    if (expected.x - backend_native.x).abs() > 1e-9 || (expected.y - backend_native.y).abs() > 1e-9
    {
        return Err("pixel_center_adapter_mismatch".into());
    }
    Ok(())
}

pub fn stroke_width_device_px(width_emu: u64, contract: &RenderRasterContractV1) -> f64 {
    let v = &contract.view;
    width_emu as f64 / v.emu_per_css_px as f64
        * (v.zoom_ppm as f64 / 1_000_000.0)
        * (v.dpr_milli as f64 / 1_000.0)
}

pub fn compatibility_v1(
    contract: &RenderRasterContractV1,
    caps: &BackendRasterCapabilitiesV1,
) -> CompatibilityV1 {
    let mut reasons = Vec::new();
    if !caps.supports_explicit_pixel_center_adapter {
        reasons.push("pixel_center_adapter_missing".into());
    }
    if !caps.supported_aa.contains(&contract.aa_class) {
        reasons.push("required_aa_class_missing".into());
    }
    if !caps
        .supported_filters
        .contains(&contract.magnification_filter)
        || !caps
            .supported_filters
            .contains(&contract.minification_filter)
    {
        reasons.push("required_sampling_filter_missing".into());
    }
    if !caps.supports_straight_alpha_sampling {
        reasons.push("straight_alpha_sampling_missing".into());
    }
    CompatibilityV1 {
        compatible: reasons.is_empty(),
        degraded: false,
        reasons,
    }
}

pub fn raster_cache_key_v1(
    material_identity: impl Into<String>,
    derivative_identity: impl Into<String>,
    mip_level: Option<u8>,
    contract: &RenderRasterContractV1,
    backend_generation: u64,
) -> RasterCacheKeyV1 {
    let device_scale_bucket = format!(
        "zoom_ppm:{}:dpr_milli:{}",
        contract.view.zoom_ppm, contract.view.dpr_milli
    );
    RasterCacheKeyV1 {
        material_identity: material_identity.into(),
        raster_contract_version: contract.contract_version.clone(),
        device_scale_bucket,
        aa_class: contract.aa_class,
        magnification_filter: contract.magnification_filter,
        minification_filter: contract.minification_filter,
        color_contract_id: contract.color_contract_id.clone(),
        backend_generation,
        derivative_identity: derivative_identity.into(),
        mip_level,
    }
}

pub fn cache_key_digest_v1(key: &RasterCacheKeyV1) -> String {
    let bytes = serde_json::to_vec(key).expect("serializable RasterCacheKeyV1");
    format!("{:x}", Sha256::digest(bytes))
}

pub fn completion_is_current_v1(completion_generation: u64, current_generation: u64) -> bool {
    completion_generation == current_generation
}

pub fn receipt_v1(contract: &RenderRasterContractV1, key: &RasterCacheKeyV1) -> RasterReceiptV1 {
    RasterReceiptV1 {
        contract_version: contract.contract_version.clone(),
        surface_generation: contract.surface.surface_generation,
        dpr_milli: contract.view.dpr_milli,
        surface_width_px: contract.surface.width_px,
        surface_height_px: contract.surface.height_px,
        aa_class: contract.aa_class,
        magnification_filter: contract.magnification_filter,
        minification_filter: contract.minification_filter,
        pixel_center: contract.pixel_center,
        cache_key_digest: cache_key_digest_v1(key),
        canonical_geometry_mutated: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view(dpr_milli: u32) -> ViewTransformV1 {
        ViewTransformV1 {
            emu_per_css_px: 9_525,
            zoom_ppm: 1_000_000,
            pan_x_milli_css_px: 0,
            pan_y_milli_css_px: 0,
            dpr_milli,
        }
    }

    fn surface(generation: u64) -> SurfaceTargetV1 {
        SurfaceTargetV1 {
            width_px: 1200,
            height_px: 800,
            surface_generation: generation,
        }
    }

    #[test]
    fn dpr_changes_device_space_not_canonical_geometry() {
        let a = baseline_contract_v1(view(1_000), surface(1), "color:v1").unwrap();
        let b = baseline_contract_v1(view(2_000), surface(2), "color:v1").unwrap();
        let canonical = (19_050_i64, 28_575_i64);
        let pa = map_emu_point_to_device(canonical.0, canonical.1, &a);
        let pb = map_emu_point_to_device(canonical.0, canonical.1, &b);
        assert_eq!(canonical, (19_050, 28_575));
        assert!((pb.x - pa.x * 2.0).abs() < 1e-9);
        assert!((pb.y - pa.y * 2.0).abs() < 1e-9);
    }

    #[test]
    fn pixel_center_adapter_is_explicit_and_wrong_half_pixel_is_rejected() {
        let c = baseline_contract_v1(view(1_000), surface(1), "color:v1").unwrap();
        let p = map_emu_point_to_device(9_525, 9_525, &c);
        let native = normalized_to_integer_center_backend(p);
        validate_pixel_center_adapter(p, native, true).unwrap();
        assert_eq!(
            validate_pixel_center_adapter(p, p, true).unwrap_err(),
            "pixel_center_adapter_mismatch"
        );
    }

    #[test]
    fn subpixel_stroke_stays_subpixel_not_hairline() {
        let c = baseline_contract_v1(view(1_000), surface(1), "color:v1").unwrap();
        let width = stroke_width_device_px(3_810, &c);
        assert!((width - 0.4).abs() < 1e-9);
        assert_ne!(width, 1.0);
        assert!(c.stroke_policy.contains("no_hairline"));
    }

    #[test]
    fn image_filters_are_explicit_and_backend_must_support_them() {
        let c = baseline_contract_v1(view(1_250), surface(1), "color:v1").unwrap();
        assert_eq!(c.magnification_filter, SamplingFilterV1::Linear);
        assert_eq!(c.minification_filter, SamplingFilterV1::Linear);
        let bad = BackendRasterCapabilitiesV1 {
            backend_id: "nearest-only".into(),
            supported_aa: vec![AaClassV1::AnalyticCoverage],
            supported_filters: vec![SamplingFilterV1::Nearest],
            supports_explicit_pixel_center_adapter: true,
            supports_straight_alpha_sampling: true,
        };
        let result = compatibility_v1(&c, &bad);
        assert!(!result.compatible);
        assert!(
            result
                .reasons
                .contains(&"required_sampling_filter_missing".into())
        );
    }

    #[test]
    fn derivative_mip_and_filter_are_distinct_cache_dimensions() {
        let c = baseline_contract_v1(view(1_500), surface(1), "color:v1").unwrap();
        let base = raster_cache_key_v1("material:a", "derivative:source", Some(0), &c, 7);
        let derivative = raster_cache_key_v1("material:a", "derivative:preview", Some(0), &c, 7);
        let mip = raster_cache_key_v1("material:a", "derivative:source", Some(1), &c, 7);
        assert_ne!(cache_key_digest_v1(&base), cache_key_digest_v1(&derivative));
        assert_ne!(cache_key_digest_v1(&base), cache_key_digest_v1(&mip));
        assert_eq!(base.magnification_filter, SamplingFilterV1::Linear);
    }

    #[test]
    fn node_identity_is_not_part_of_shareable_material_key() {
        let c = baseline_contract_v1(view(2_000), surface(1), "color:v1").unwrap();
        let a = raster_cache_key_v1("sha256:material", "source", None, &c, 1);
        let b = raster_cache_key_v1("sha256:material", "source", None, &c, 1);
        assert_eq!(a, b);
        assert!(!serde_json::to_string(&a).unwrap().contains("node_id"));
    }

    #[test]
    fn stale_dpr_or_backend_generation_completion_is_fenced() {
        assert!(completion_is_current_v1(9, 9));
        assert!(!completion_is_current_v1(8, 9));
    }

    #[test]
    fn backend_rebuild_recreates_same_policy_but_new_physical_generation() {
        let c = baseline_contract_v1(view(2_000), surface(11), "color:v1").unwrap();
        let key11 = raster_cache_key_v1("material", "source", None, &c, 11);
        let key12 = raster_cache_key_v1("material", "source", None, &c, 12);
        assert_eq!(key11.raster_contract_version, key12.raster_contract_version);
        assert_eq!(key11.device_scale_bucket, key12.device_scale_bucket);
        assert_ne!(key11.backend_generation, key12.backend_generation);
    }

    #[test]
    fn compatible_backend_must_adapt_all_mandatory_raster_facts() {
        let c = baseline_contract_v1(view(1_000), surface(1), "color:v1").unwrap();
        let caps = BackendRasterCapabilitiesV1 {
            backend_id: "reference".into(),
            supported_aa: vec![AaClassV1::AnalyticCoverage],
            supported_filters: vec![SamplingFilterV1::Linear],
            supports_explicit_pixel_center_adapter: true,
            supports_straight_alpha_sampling: true,
        };
        assert!(compatibility_v1(&c, &caps).compatible);
    }
}
