use chaptera_render_raster_contract::{
    SurfaceTargetV1, ViewTransformV1, baseline_contract_v1, raster_cache_key_v1, receipt_v1,
};

fn main() {
    let contract = baseline_contract_v1(
        ViewTransformV1 {
            emu_per_css_px: 9_525,
            zoom_ppm: 1_000_000,
            pan_x_milli_css_px: 0,
            pan_y_milli_css_px: 0,
            dpr_milli: 1_500,
        },
        SurfaceTargetV1 {
            width_px: 1800,
            height_px: 1200,
            surface_generation: 3,
        },
        "chaptera.render-color.v1:srgb-straight",
    )
    .expect("valid contract");
    let key = raster_cache_key_v1(
        "sha256:material",
        "derivative:source",
        Some(0),
        &contract,
        7,
    );
    println!(
        "{}",
        serde_json::to_string_pretty(&receipt_v1(&contract, &key)).unwrap()
    );
}
