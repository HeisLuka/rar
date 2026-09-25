use chaptera_glyph_runtime::{
    DeterministicTestMaterializer, FontRuntimeState, GLYPH_RUNTIME_BENCHMARK_SCHEMA_V1,
    GlyphRuntime, RenderFontResourceV1, RenderGlyphRunV1, RenderGlyphV1,
};
use serde_json::json;
use std::time::Instant;

fn text_heavy_runs(count: usize) -> (Vec<RenderGlyphRunV1>, RenderFontResourceV1) {
    let font = RenderFontResourceV1 {
        resource_id: "font:synthetic".into(),
        kind: "font".into(),
        content_hash: "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff".into(),
    };
    let runs = (0..count).map(|index| RenderGlyphRunV1 {
        page_id: format!("page:{}", index % 5),
        story_id: format!("story:{index}"),
        frame_node_id: format!("frame:{index}"),
        scalar_start: 0,
        scalar_end: 120,
        font_resource_id: font.resource_id.clone(),
        paint_id: Some("paint:solid".into()),
        glyphs: (0..24).map(|glyph| RenderGlyphV1 {
            glyph_id: 65 + (glyph % 26),
            x_emu: 100_000 + i64::from(glyph) * 40_000,
            y_emu: 120_000,
            advance_emu: 40_000,
        }).collect(),
    }).collect();
    (runs, font)
}

fn elapsed_ms(start: Instant) -> f64 {
    start.elapsed().as_secs_f64() * 1000.0
}

fn main() {
    let (runs, font) = text_heavy_runs(250);
    let placements: usize = runs.iter().map(|run| run.glyphs.len()).sum();
    let mut runtime = GlyphRuntime::new(
        DeterministicTestMaterializer::default(),
        128,
        4,
        1,
    ).expect("runtime");
    runtime.set_font_state(font.content_hash.clone(), FontRuntimeState::Ready, 1)
        .expect("font");

    let first_start = Instant::now();
    runtime.prepare_run(&runs[0], &font, 0, 1.0, 1.0).expect("first run");
    let first_visible_text_ms = elapsed_ms(first_start);

    let cold_start = Instant::now();
    for run in &runs[1..] {
        runtime.prepare_run(run, &font, 0, 1.0, 1.0).expect("cold");
    }
    let cold_ms = elapsed_ms(cold_start);
    let cold = runtime.receipt();

    let warm_start = Instant::now();
    for run in &runs {
        runtime.prepare_run(run, &font, 0, 1.0, 1.0).expect("warm");
    }
    let warm_ms = elapsed_ms(warm_start);
    let warm = runtime.receipt();

    let transition_start = Instant::now();
    for run in &runs {
        runtime.prepare_run(run, &font, 0, 4.0, 2.0).expect("transition");
    }
    let zoom_dpr_transition_ms = elapsed_ms(transition_start);
    let after_transition = runtime.receipt();

    let vector_start = Instant::now();
    for run in &runs {
        runtime.prepare_run(run, &font, 0, 9.0, 1.0).expect("vector");
    }
    let high_zoom_vector_ms = elapsed_ms(vector_start);
    let high_zoom = runtime.receipt();

    let output = json!({
        "schema": GLYPH_RUNTIME_BENCHMARK_SCHEMA_V1,
        "measurement_class": "synthetic_source_neutral_text_heavy_render_scene_shape",
        "real_pub": false,
        "representative_corpus": false,
        "technology_decision_allowed": false,
        "materializer": "deterministic_test_materializer_not_real_rasterizer",
        "glyph_placements": placements,
        "unique_input_glyph_ids": 24,
        "first_visible_text_ms": first_visible_text_ms,
        "cold_ms": cold_ms,
        "warm_ms": warm_ms,
        "zoom_dpr_transition_ms": zoom_dpr_transition_ms,
        "high_zoom_vector_ms": high_zoom_vector_ms,
        "cold": cold,
        "warm": warm,
        "after_transition": after_transition,
        "high_zoom": high_zoom,
        "gpu_texture_resident_bytes": null,
        "limitations": [
            "Deterministic materializer proves cache/residency lifecycle only; it is not raster quality or GPU timing evidence.",
            "No host-font discovery and no reshaping occur.",
            "No genuine PUB-derived RenderScene receipt is currently available on main."
        ]
    });
    println!("{}", serde_json::to_string_pretty(&output).unwrap());
}
