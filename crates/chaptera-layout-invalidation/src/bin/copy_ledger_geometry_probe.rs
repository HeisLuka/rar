use chaptera_layout_invalidation::runtime::{
    FrameGeometryV1, IntervalPolicyV1, ResolvedScalarMetricV1, layout_copy_ledger_snapshot_v1,
    prepare_from_resolved_metrics_v1, reset_layout_copy_ledger_v1, resolve_line_regions_v1,
    resolve_story_flow_v1,
};

fn metrics(start: u32) -> Vec<ResolvedScalarMetricV1> {
    (0..3)
        .map(|offset| ResolvedScalarMetricV1 {
            scalar_start: start + offset,
            scalar_end: start + offset + 1,
            advance_emu: 20,
            break_after: offset == 1,
            semantic_fingerprint: [u8::try_from(start + offset).unwrap_or(0); 32],
        })
        .collect()
}

fn main() {
    let first = prepare_from_resolved_metrics_v1(
        "story:copy-ledger",
        "unit:1",
        metrics(0),
        [1; 32],
        [2; 32],
    )
    .expect("prepare first unit");
    let second = prepare_from_resolved_metrics_v1(
        "story:copy-ledger",
        "unit:2",
        metrics(3),
        [1; 32],
        [2; 32],
    )
    .expect("prepare second unit");
    let prepared = vec![first, second];

    let frame = FrameGeometryV1 {
        frame_id: "frame:copy-ledger".to_owned(),
        page_id: "page:1".to_owned(),
        width_emu: 60,
        height_emu: 120,
        line_height_emu: 20,
    };
    let region = resolve_line_regions_v1(&frame, &[]).expect("resolve geometry region");

    reset_layout_copy_ledger_v1();
    let flow = resolve_story_flow_v1(
        "story:copy-ledger",
        &prepared,
        &region,
        IntervalPolicyV1::LargestOnly,
    )
    .expect("geometry-only reflow");
    let snapshot = layout_copy_ledger_snapshot_v1();

    println!(
        "{{"scenario":"geometry_only_reflow","lines":{},"prepared_units_clone_bytes":{},"prepared_units_clone_instances":{}}}",
        flow.lines.len(),
        snapshot.prepared_units_clone_bytes,
        snapshot.prepared_units_clone_instances,
    );
}
