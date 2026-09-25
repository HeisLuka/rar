use chaptera_glyph_runtime::{
    DeterministicTestMaterializer, FontRuntimeState, GlyphCacheKey, GlyphQuality, GlyphRenderMode,
    GlyphRequestResult, GlyphRuntime, RenderFontResourceV1, RenderGlyphRunV1,
    select_material_policy,
};

const FONT_A: &str = "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff";
const FONT_B: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

fn run() -> RenderGlyphRunV1 {
    serde_json::from_str(
        r#"{
      "atom_id":"node:0:glyph_run",
      "node_id":"node",
      "page_id":"page",
      "story_id":"story",
      "frame_node_id":"node",
      "scalar_start":0,
      "scalar_end":5,
      "font_resource_id":"font:1",
      "paint_id":"paint:solid",
      "glyphs":[
        {"glyph_id":10,"x_emu":110000,"y_emu":910000,"advance_emu":70000},
        {"glyph_id":11,"x_emu":180000,"y_emu":910000,"advance_emu":70000},
        {"glyph_id":10,"x_emu":250000,"y_emu":910000,"advance_emu":70000}
      ],
      "effect_group_id":null,
      "clip_id":null
    }"#,
    )
    .unwrap()
}

fn font(hash: &str) -> RenderFontResourceV1 {
    RenderFontResourceV1 {
        resource_id: "font:1".into(),
        kind: "font".into(),
        content_hash: hash.into(),
    }
}

fn runtime(page_capacity: u32, max_pages: u32) -> GlyphRuntime<DeterministicTestMaterializer> {
    let mut runtime = GlyphRuntime::new(
        DeterministicTestMaterializer::default(),
        page_capacity,
        max_pages,
        1,
    )
    .unwrap();
    runtime
        .set_font_state(FONT_A, FontRuntimeState::Ready, 1)
        .unwrap();
    runtime
}

fn key(hash: &str, face: u32, glyph: u32, bucket: u16) -> GlyphCacheKey {
    GlyphCacheKey {
        font_fingerprint: hash.into(),
        face_index: face,
        glyph_id: glyph,
        render_mode: GlyphRenderMode::Raster,
        scale_bucket: bucket,
        quality: GlyphQuality::Normal,
    }
}

#[test]
fn current_render_scene_json_shape_deserializes_without_typography_reinterpretation() {
    let value = run();
    assert_eq!(value.story_id, "story");
    assert_eq!(value.frame_node_id, "node");
    assert_eq!(value.scalar_start, 0);
    assert_eq!(value.scalar_end, 5);
    assert_eq!(value.glyphs[0].glyph_id, 10);
    assert_eq!(value.glyphs[0].x_emu, 110000);
    assert_eq!(value.glyphs[0].advance_emu, 70000);
}

#[test]
fn repeated_placements_reuse_one_exact_materialization() {
    let mut runtime = runtime(8, 2);
    let prepared = runtime
        .prepare_run(&run(), &font(FONT_A), 0, 1.0, 1.0)
        .unwrap();
    assert_eq!(prepared.geometry, run().glyphs);
    assert!(!prepared.canonical_geometry_mutated);
    // glyph 10 is placed twice; only glyph 10 + 11 should materialize.
    assert_eq!(runtime.materializer().calls, 2);
    assert_eq!(runtime.receipt().unique_logical_entries, 2);

    let again = runtime
        .prepare_run(&run(), &font(FONT_A), 0, 1.0, 1.0)
        .unwrap();
    assert_eq!(again.geometry, prepared.geometry);
    assert_eq!(runtime.materializer().calls, 2);
    assert!(runtime.receipt().metrics.hits >= 4);
}

#[test]
fn font_fingerprint_face_and_bucket_never_alias() {
    let mut runtime = runtime(8, 2);
    runtime
        .set_font_state(FONT_B, FontRuntimeState::Ready, 1)
        .unwrap();
    let keys = [
        key(FONT_A, 0, 10, 1),
        key(FONT_B, 0, 10, 1),
        key(FONT_A, 1, 10, 1),
        key(FONT_A, 0, 10, 2),
    ];
    let bindings = keys
        .iter()
        .map(|key| match runtime.request(key.clone()).unwrap() {
            GlyphRequestResult::Ready { binding } => binding,
            other => panic!("unexpected result: {other:?}"),
        })
        .collect::<Vec<_>>();
    assert_eq!(runtime.receipt().unique_logical_entries, 4);
    assert_eq!(
        bindings
            .iter()
            .map(|b| b.key.clone())
            .collect::<std::collections::BTreeSet<_>>()
            .len(),
        4
    );
}

#[test]
fn scale_bucket_transition_changes_only_material_identity_not_geometry() {
    let mut runtime = runtime(32, 2);
    let input = run();
    let normal = runtime
        .prepare_run(&input, &font(FONT_A), 0, 1.0, 1.0)
        .unwrap();
    let high = runtime
        .prepare_run(&input, &font(FONT_A), 0, 4.0, 1.0)
        .unwrap();
    assert_eq!(normal.geometry, input.glyphs);
    assert_eq!(high.geometry, input.glyphs);
    assert_eq!(normal.scale_bucket, 1);
    assert_eq!(high.scale_bucket, 4);
    assert_ne!(normal.scale_bucket, high.scale_bucket);
}

#[test]
fn high_zoom_uses_vector_fidelity_instead_of_magnifying_low_res_raster_forever() {
    let at_eight = select_material_policy(8.0, 1.0).unwrap();
    let beyond = select_material_policy(8.01, 1.0).unwrap();
    assert_eq!(at_eight.render_mode, GlyphRenderMode::Raster);
    assert_eq!(at_eight.scale_bucket, 8);
    assert_eq!(beyond.render_mode, GlyphRenderMode::Vector);
    assert_eq!(beyond.quality, GlyphQuality::Fidelity);
}

#[test]
fn eviction_and_rebuild_preserve_logical_key_and_reject_stale_binding() {
    let mut runtime = runtime(4, 1);
    let logical = key(FONT_A, 0, 10, 1);
    let first = match runtime.request(logical.clone()).unwrap() {
        GlyphRequestResult::Ready { binding } => binding,
        other => panic!("{other:?}"),
    };
    runtime.evict(&logical);
    assert!(!runtime.validate_binding(&first));
    let second = match runtime.request(logical.clone()).unwrap() {
        GlyphRequestResult::Ready { binding } => binding,
        other => panic!("{other:?}"),
    };
    assert_eq!(first.key, second.key);
    assert_ne!(first.residency_generation, second.residency_generation);
    assert!(runtime.validate_binding(&second));
}

#[test]
fn atlas_repack_moves_physical_residency_without_changing_semantic_keys() {
    let mut runtime = runtime(4, 2);
    let mut old = Vec::new();
    for glyph in 10..15 {
        let binding = match runtime.request(key(FONT_A, 0, glyph, 1)).unwrap() {
            GlyphRequestResult::Ready { binding } => binding,
            other => panic!("{other:?}"),
        };
        old.push(binding);
    }
    runtime.evict(&key(FONT_A, 0, 11, 1));
    let keys_before = old
        .iter()
        .map(|b| b.key.clone())
        .collect::<std::collections::BTreeSet<_>>();
    runtime.repack();
    assert!(old.iter().any(|binding| !runtime.validate_binding(binding)));
    let receipt = runtime.receipt();
    assert_eq!(receipt.authority.atlas_slot_is_semantic_identity, false);
    // Logical cache entries survive repack/eviction even though residency moved.
    assert!(receipt.unique_logical_entries >= keys_before.len());
}

#[test]
fn bounded_atlas_automatically_evicts_lru_without_document_mutation() {
    let mut runtime = runtime(2, 1);
    for glyph in 10..14 {
        let result = runtime.request(key(FONT_A, 0, glyph, 1)).unwrap();
        assert!(matches!(result, GlyphRequestResult::Ready { .. }));
    }
    let receipt = runtime.receipt();
    assert_eq!(receipt.atlas.capacity_slots, 2);
    assert_eq!(receipt.resident_entries, 2);
    assert_eq!(receipt.metrics.automatic_evictions, 2);
    assert!(!receipt.authority.mutates_canonical_glyph_positions);
}

#[test]
fn font_generation_change_invalidates_old_residency_without_changing_font_identity() {
    let mut runtime = runtime(4, 1);
    let logical = key(FONT_A, 0, 10, 1);
    let old = match runtime.request(logical.clone()).unwrap() {
        GlyphRequestResult::Ready { binding } => binding,
        other => panic!("{other:?}"),
    };
    runtime
        .set_font_state(FONT_A, FontRuntimeState::Ready, 2)
        .unwrap();
    assert!(!runtime.validate_binding(&old));
    let new = match runtime.request(logical.clone()).unwrap() {
        GlyphRequestResult::Ready { binding } => binding,
        other => panic!("{other:?}"),
    };
    assert_eq!(old.key, new.key);
    assert_eq!(new.font_generation, 2);
}

#[test]
fn pending_blocked_failed_fonts_are_explicit_and_never_host_substituted() {
    let mut runtime = GlyphRuntime::new(DeterministicTestMaterializer::default(), 4, 1, 1).unwrap();
    let logical = key(FONT_A, 0, 10, 1);
    assert!(matches!(
        runtime.request(logical.clone()).unwrap(),
        GlyphRequestResult::Pending { .. }
    ));
    runtime
        .set_font_state(FONT_A, FontRuntimeState::Blocked, 1)
        .unwrap();
    assert!(matches!(
        runtime.request(logical.clone()).unwrap(),
        GlyphRequestResult::Blocked { .. }
    ));
    runtime
        .set_font_state(FONT_A, FontRuntimeState::Failed, 1)
        .unwrap();
    assert!(matches!(
        runtime.request(logical.clone()).unwrap(),
        GlyphRequestResult::Failed { .. }
    ));
    assert_eq!(runtime.materializer().calls, 0);
    assert!(!runtime.receipt().authority.discovers_host_fonts);
}

#[test]
fn device_loss_rebuilds_disposable_residency_and_old_binding_fails_closed() {
    let mut runtime = runtime(4, 1);
    let logical = key(FONT_A, 0, 10, 1);
    let old = match runtime.request(logical.clone()).unwrap() {
        GlyphRequestResult::Ready { binding } => binding,
        other => panic!("{other:?}"),
    };
    runtime.reset_device();
    assert!(!runtime.validate_binding(&old));
    let new = match runtime.request(logical).unwrap() {
        GlyphRequestResult::Ready { binding } => binding,
        other => panic!("{other:?}"),
    };
    assert_eq!(new.device_generation, 2);
    assert!(runtime.validate_binding(&new));
}

#[test]
fn unsupported_complex_color_path_is_explicit() {
    let mut runtime = runtime(4, 1);
    let mut logical = key(FONT_A, 0, 10, 1);
    logical.render_mode = GlyphRenderMode::ColorComplex;
    assert!(matches!(
        runtime.request(logical).unwrap(),
        GlyphRequestResult::Unsupported { .. }
    ));
}

#[test]
fn wrong_resource_or_non_font_resource_fails_closed() {
    let mut runtime = runtime(4, 1);
    let mut wrong = font(FONT_A);
    wrong.resource_id = "font:other".into();
    assert!(runtime.prepare_run(&run(), &wrong, 0, 1.0, 1.0).is_err());

    let mut image = font(FONT_A);
    image.kind = "image".into();
    assert!(runtime.prepare_run(&run(), &image, 0, 1.0, 1.0).is_err());
}
