use std::collections::BTreeMap;

use chaptera_layout_invalidation::linked_flow::{
    ContinuationTerminalV1, LinkedFrameInputV1, resolve_linked_story_full_v1,
    resolve_linked_story_incremental_v1,
};
use chaptera_layout_invalidation::prepared_paragraph::prepare_projected_story_v1;
use chaptera_layout_invalidation::runtime::{
    FrameGeometryV1, IntervalPolicyV1, ResolvedScalarMetricV1, resolve_line_regions_v1,
};
use chaptera_layout_invalidation::{FingerprintV1, fingerprint_v1};
use chaptera_layout_projection::{
    ResolvedShapingRunInputV1, StoryProjectionInputV1, project_story_text_v1,
};
use pub_fixed_flow_adapter::{
    ProducerV1, ResolvedGlyphV1, ShapedFlowInputV1, ShapedLineInputV1, build_receipt_v1,
    materialize_fixed_runs_v1, run_local_cluster_v1,
};
use pub_model::{derive_pub_node_id_v1, derive_pub_story_id_v1};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct Fixture {
    schema: String,
    source: Source,
    story: Story,
    frames: Vec<Frame>,
    wrap_obstacle_refs: BTreeMap<String, Vec<u32>>,
    benchmark_harness: Harness,
}

#[derive(Debug, Deserialize)]
struct Source {
    sha256: String,
    size_bytes: u64,
}

#[derive(Debug, Deserialize)]
struct Story {
    text_id: u32,
    utf16_code_units: u32,
    unicode_scalar_count: u32,
    paragraph_boundary_count: u32,
    text: String,
}

#[derive(Debug, Deserialize)]
struct Frame {
    seq_num: u32,
    effective_ordinal: u32,
    raw_explicit_ordinal: Option<u32>,
    previous_seq_num: Option<u32>,
    next_seq_num: Option<u32>,
    bounds_emu: Bounds,
}

#[derive(Debug, Deserialize)]
struct Bounds {
    x: i64,
    y: i64,
    width: i64,
    height: i64,
}

#[derive(Debug, Deserialize)]
struct Harness {
    authority: String,
    synthetic_advance_emu_per_non_cr_scalar: i64,
    synthetic_line_height_emu: i64,
    wrap_obstacles_materialized: bool,
    baseline_expected_frame_end_scalars: Vec<u32>,
    mutation: Mutation,
    mutated_expected_frame_end_scalars: Vec<u32>,
    expected_convergence_after_seq_num: u32,
    expected_reused_suffix_from_seq_num: u32,
}

#[derive(Debug, Deserialize)]
struct Mutation {
    frame_330_width_delta_emu: i64,
    frame_329_width_delta_emu: i64,
}

fn fixture() -> Fixture {
    serde_json::from_str(include_str!(
        "fixtures/sample_newsletter_story22_real_v1.json"
    ))
    .expect("real SampleNewsletter Story22 fixture")
}

fn fp(domain: &str, value: &str) -> FingerprintV1 {
    fingerprint_v1(domain, &[value.as_bytes()])
}

fn build_prepared(
    fixture: &Fixture,
) -> chaptera_layout_invalidation::prepared_paragraph::PreparedStoryIndexV1 {
    let scalar_count =
        u32::try_from(fixture.story.text.chars().count()).expect("bounded real fixture");
    assert_eq!(scalar_count, fixture.story.unicode_scalar_count);

    let paragraph_count = fixture.story.text.chars().filter(|ch| *ch == '\r').count() + 1;
    let paragraph_ids = (0..paragraph_count)
        .map(|index| format!("sample-newsletter-story22-p{index}"))
        .collect::<Vec<_>>();

    let projected = project_story_text_v1(StoryProjectionInputV1 {
        story_id: "sample-newsletter-story22".into(),
        text: fixture.story.text.clone(),
        paragraph_ids,
        shaping_runs: vec![ResolvedShapingRunInputV1 {
            scalar_start: 0,
            scalar_end: scalar_count,
            resolved_shaping_fingerprint: "synthetic-fixed-metrics-v1".into(),
        }],
    });
    assert!(projected.is_usable_for_preparation());

    let metrics = fixture
        .story
        .text
        .chars()
        .enumerate()
        .filter_map(|(index, ch)| {
            if ch == '\r' {
                return None;
            }

            let scalar = u32::try_from(index).expect("bounded real fixture");
            let scalar_bytes = scalar.to_be_bytes();
            let mut encoded = [0_u8; 4];
            let char_bytes = ch.encode_utf8(&mut encoded).as_bytes();

            Some(ResolvedScalarMetricV1 {
                scalar_start: scalar,
                scalar_end: scalar + 1,
                advance_emu: fixture
                    .benchmark_harness
                    .synthetic_advance_emu_per_non_cr_scalar,
                break_after: ch.is_whitespace(),
                semantic_fingerprint: fingerprint_v1(
                    "sample-newsletter-story22-scalar-v1",
                    &[&scalar_bytes, char_bytes],
                ),
            })
        })
        .collect::<Vec<_>>();

    prepare_projected_story_v1(
        &projected,
        &metrics,
        fp("sample-newsletter-layout-env-v1", "synthetic-fixed-font"),
        fp(
            "sample-newsletter-shaping-policy-v1",
            "synthetic-fixed-metrics",
        ),
    )
    .expect("prepare real Story with explicitly synthetic metrics")
}

fn build_frames(fixture: &Fixture, mutated: bool) -> Vec<LinkedFrameInputV1> {
    fixture
        .frames
        .iter()
        .map(|frame| {
            let width_delta = if mutated {
                match frame.seq_num {
                    330 => fixture.benchmark_harness.mutation.frame_330_width_delta_emu,
                    329 => fixture.benchmark_harness.mutation.frame_329_width_delta_emu,
                    _ => 0,
                }
            } else {
                0
            };
            let frame_id = format!("seq-{}", frame.seq_num);
            let geometry = FrameGeometryV1 {
                frame_id: frame_id.clone(),
                page_id: "sample-newsletter-page".into(),
                width_emu: frame.bounds_emu.width + width_delta,
                height_emu: frame.bounds_emu.height,
                line_height_emu: fixture.benchmark_harness.synthetic_line_height_emu,
            };
            let region = resolve_line_regions_v1(&geometry, &[]).expect("real frame region");

            LinkedFrameInputV1 {
                frame_id,
                previous_frame_id: frame.previous_seq_num.map(|seq| format!("seq-{seq}")),
                next_frame_id: frame.next_seq_num.map(|seq| format!("seq-{seq}")),
                region,
            }
        })
        .collect()
}

fn frame_end_scalars(
    flow: &chaptera_layout_invalidation::linked_flow::LinkedStoryFlowV1,
) -> Vec<u32> {
    flow.frames
        .iter()
        .map(|frame| frame.output_boundary.continuation.next_scalar)
        .collect()
}


fn shaped_flow_input(
    fixture: &Fixture,
    flow: &chaptera_layout_invalidation::linked_flow::LinkedStoryFlowV1,
) -> ShapedFlowInputV1 {
    let chars = fixture.story.text.chars().collect::<Vec<_>>();
    let story_id =
        derive_pub_story_id_v1(&fixture.source.sha256, fixture.story.text_id).expect("Story UUID");
    let mut lines = Vec::new();

    for frame in &flow.frames {
        let seq_num = frame
            .frame_id
            .strip_prefix("seq-")
            .expect("fixture frame prefix")
            .parse::<u32>()
            .expect("fixture seqNum");
        let frame_node_id =
            derive_pub_node_id_v1(&fixture.source.sha256, seq_num).expect("frame UUID");

        for line in &frame.lines {
            let glyphs = (line.scalar_start..line.scalar_end)
                .map(|scalar| {
                    let ch = chars[usize::try_from(scalar).expect("bounded fixture")];
                    assert_ne!(ch, '\r');

                    ResolvedGlyphV1 {
                        // This is an explicit bounded materialization oracle, not
                        // a Publisher font glyph ID. One canonical scalar maps to
                        // one stable synthetic glyph identity for this test only.
                        glyph_id: u32::from(ch),
                        cluster: scalar,
                        x_advance: fixture
                            .benchmark_harness
                            .synthetic_advance_emu_per_non_cr_scalar,
                        y_advance: 0,
                        x_offset: 0,
                        y_offset: 0,
                    }
                })
                .collect::<Vec<_>>();

            let measured_width = u64::try_from(line.measured_width_emu)
                .expect("non-negative bounded line width");
            let expected_width = u64::try_from(
                fixture
                    .benchmark_harness
                    .synthetic_advance_emu_per_non_cr_scalar,
            )
            .expect("positive synthetic advance")
                * u64::try_from(glyphs.len()).expect("bounded glyph count");
            assert_eq!(measured_width, expected_width);

            lines.push(ShapedLineInputV1 {
                line_index: lines.len(),
                frame_line_index: line.row_index,
                frame_node_id: frame_node_id.clone(),
                story_id: story_id.clone(),
                scalar_start: line.scalar_start,
                scalar_end: line.scalar_end,
                units_per_em: 1000,
                measured_width,
                glyphs,
            });
        }
    }

    ShapedFlowInputV1 {
        producer: ProducerV1 {
            implementation: "rar-linked-flow-materialization".into(),
            commit_or_build: "sample-newsletter-story22-real-v1".into(),
            core_integration: true,
        },
        source_hash: fixture.source.sha256.clone(),
        font_size_emu: 250_000,
        line_height_emu: fixture.benchmark_harness.synthetic_line_height_emu,
        story_overset: flow
            .frames
            .last()
            .is_some_and(|frame| {
                frame.output_boundary.continuation.terminal == ContinuationTerminalV1::Overset
            }),
        lines,
    }
}

fn frame_line_receipts(
    receipt: &pub_fixed_flow_adapter::ShapedFlowReceiptV1,
    frame_node_id: &str,
) -> Vec<pub_fixed_flow_adapter::LineReceiptV1> {
    receipt
        .lines
        .iter()
        .filter(|line| line.frame_node_id == frame_node_id)
        .cloned()
        .collect()
}

fn frame_run_receipts(
    receipt: &pub_fixed_flow_adapter::ShapedFlowReceiptV1,
    frame_node_id: &str,
) -> Vec<pub_fixed_flow_adapter::RunReceiptV1> {
    receipt
        .runs
        .iter()
        .filter(|run| run.frame_node_id == frame_node_id)
        .cloned()
        .collect()
}

#[test]
fn real_sample_newsletter_story22_incremental_flow_converges_to_clean_recompute() {
    let fixture = fixture();

    assert_eq!(fixture.schema, "chaptera.sample-newsletter-story22-real.v1");
    assert_eq!(
        fixture.source.sha256,
        "6a825ba26ba35d6e885acdc62e859591ed37cb0ff7480b554b9cb362b644dfcf"
    );
    assert_eq!(fixture.source.size_bytes, 291_840);
    assert_eq!(fixture.story.text_id, 22);
    assert_eq!(fixture.story.utf16_code_units, 601);
    assert_eq!(fixture.story.unicode_scalar_count, 601);
    assert_eq!(fixture.story.paragraph_boundary_count, 5);
    assert_eq!(
        fixture.story.text.chars().filter(|ch| *ch == '\r').count(),
        5
    );
    assert!(
        fixture
            .benchmark_harness
            .authority
            .contains("synthetic typography only")
    );
    assert!(!fixture.benchmark_harness.wrap_obstacles_materialized);

    assert_eq!(
        fixture
            .frames
            .iter()
            .map(|frame| frame.seq_num)
            .collect::<Vec<_>>(),
        vec![330, 329, 331]
    );
    assert_eq!(fixture.frames[0].effective_ordinal, 0);
    assert_eq!(fixture.frames[0].raw_explicit_ordinal, None);
    assert_eq!(fixture.frames[1].raw_explicit_ordinal, Some(1));
    assert_eq!(fixture.frames[2].raw_explicit_ordinal, Some(2));
    assert_eq!(fixture.frames[0].previous_seq_num, None);
    assert_eq!(fixture.frames[0].next_seq_num, Some(329));
    assert_eq!(fixture.frames[1].previous_seq_num, Some(330));
    assert_eq!(fixture.frames[1].next_seq_num, Some(331));
    assert_eq!(fixture.frames[2].previous_seq_num, Some(329));
    assert_eq!(fixture.frames[2].next_seq_num, None);

    assert_eq!(
        fixture
            .frames
            .iter()
            .map(|frame| (
                frame.bounds_emu.x,
                frame.bounds_emu.y,
                frame.bounds_emu.width,
                frame.bounds_emu.height
            ))
            .collect::<Vec<_>>(),
        vec![
            (2_596_806, 7_930_671, 1_360_324, 2_059_945),
            (4_134_862, 7_930_671, 1_360_325, 2_059_945),
            (5_672_918, 7_930_671, 1_360_324, 2_059_945),
        ]
    );
    assert_eq!(fixture.wrap_obstacle_refs.get("330"), Some(&vec![]));
    assert_eq!(fixture.wrap_obstacle_refs.get("329"), Some(&vec![337, 338]));
    assert_eq!(fixture.wrap_obstacle_refs.get("331"), Some(&vec![337, 338]));

    let prepared = build_prepared(&fixture);
    let environment = fp("sample-newsletter-layout-env-v1", "synthetic-fixed-font");
    let flow_policy = fp(
        "sample-newsletter-flow-policy-v1",
        "largest-real-frame-regions",
    );

    let baseline_frames = build_frames(&fixture, false);
    let baseline = resolve_linked_story_full_v1(
        &prepared,
        &baseline_frames,
        environment,
        flow_policy,
        IntervalPolicyV1::LargestOnly,
    )
    .expect("baseline real-derived linked Story flow");

    assert_eq!(
        frame_end_scalars(&baseline),
        fixture
            .benchmark_harness
            .baseline_expected_frame_end_scalars
    );
    assert_eq!(
        baseline
            .frames
            .last()
            .expect("third real frame")
            .output_boundary
            .continuation
            .terminal,
        ContinuationTerminalV1::Complete
    );

    let mutated_frames = build_frames(&fixture, true);
    let incremental = resolve_linked_story_incremental_v1(
        &prepared,
        &mutated_frames,
        &baseline,
        "seq-330",
        environment,
        flow_policy,
        IntervalPolicyV1::LargestOnly,
    )
    .expect("incremental real-derived linked Story flow");
    let clean = resolve_linked_story_full_v1(
        &prepared,
        &mutated_frames,
        environment,
        flow_policy,
        IntervalPolicyV1::LargestOnly,
    )
    .expect("clean recompute");

    assert_eq!(incremental.flow, clean);
    assert_eq!(
        frame_end_scalars(&incremental.flow),
        fixture.benchmark_harness.mutated_expected_frame_end_scalars
    );
    assert_eq!(incremental.recomputed_frame_ids, vec!["seq-330", "seq-329"]);
    assert_eq!(
        incremental.convergence_after_frame_id.as_deref(),
        Some("seq-329")
    );
    assert_eq!(
        incremental.reused_suffix_from_frame_id.as_deref(),
        Some("seq-331")
    );
    assert_eq!(
        incremental.convergence_after_frame_id,
        Some(format!(
            "seq-{}",
            fixture.benchmark_harness.expected_convergence_after_seq_num
        ))
    );
    assert_eq!(
        incremental.reused_suffix_from_frame_id,
        Some(format!(
            "seq-{}",
            fixture
                .benchmark_harness
                .expected_reused_suffix_from_seq_num
        ))
    );

    assert_ne!(
        baseline.frames[0].output_boundary.continuation.next_scalar,
        incremental.flow.frames[0]
            .output_boundary
            .continuation
            .next_scalar
    );
    assert_eq!(
        baseline.frames[1].output_boundary,
        incremental.flow.frames[1].output_boundary
    );
    assert_eq!(
        baseline.frames[2].output_fingerprint,
        incremental.flow.frames[2].output_fingerprint
    );

    let baseline_shaped = shaped_flow_input(&fixture, &baseline);
    let incremental_shaped = shaped_flow_input(&fixture, &incremental.flow);
    let clean_shaped = shaped_flow_input(&fixture, &clean);

    assert_eq!(incremental_shaped, clean_shaped);

    let baseline_runs =
        materialize_fixed_runs_v1(&baseline_shaped).expect("baseline fixed-run materialization");
    let incremental_runs = materialize_fixed_runs_v1(&incremental_shaped)
        .expect("incremental fixed-run materialization");
    let clean_runs =
        materialize_fixed_runs_v1(&clean_shaped).expect("clean fixed-run materialization");

    assert_eq!(incremental_runs, clean_runs);

    for run in &incremental_runs {
        let logical_scalar_len = run.scalar_end - run.scalar_base;
        for glyph in &run.glyphs {
            assert_eq!(
                run_local_cluster_v1(
                    run.run_index,
                    glyph.cluster,
                    run.scalar_base,
                    logical_scalar_len
                )
                .expect("Story-global cluster must rebase inside materialized run"),
                glyph.cluster - run.scalar_base
            );
        }
    }

    let baseline_receipt = build_receipt_v1(&baseline_shaped).expect("baseline receipt");
    let incremental_receipt =
        build_receipt_v1(&incremental_shaped).expect("incremental receipt");
    let clean_receipt = build_receipt_v1(&clean_shaped).expect("clean receipt");

    assert_eq!(incremental_receipt, clean_receipt);
    assert_ne!(baseline_receipt.flow_id, incremental_receipt.flow_id);
    assert_eq!(incremental_receipt.invariants.reshaping_calls, 0);
    assert!(!incremental_receipt.invariants.raw_text_emitted);
    assert!(incremental_receipt.invariants.story_global_clusters_preserved);
    assert!(incremental_receipt.invariants.line_order_preserved);

    let reused_frame_id =
        derive_pub_node_id_v1(&fixture.source.sha256, 331).expect("seq331 source UUID");
    assert_eq!(
        frame_line_receipts(&baseline_receipt, &reused_frame_id),
        frame_line_receipts(&incremental_receipt, &reused_frame_id)
    );
    assert_eq!(
        frame_run_receipts(&baseline_receipt, &reused_frame_id),
        frame_run_receipts(&incremental_receipt, &reused_frame_id)
    );

    let changed_frame_id =
        derive_pub_node_id_v1(&fixture.source.sha256, 330).expect("seq330 source UUID");
    assert_ne!(
        frame_line_receipts(&baseline_receipt, &changed_frame_id),
        frame_line_receipts(&incremental_receipt, &changed_frame_id)
    );

}
