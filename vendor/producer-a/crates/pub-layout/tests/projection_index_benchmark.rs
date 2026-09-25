use std::{env, fs, path::Path, time::Instant};

use pub_layout::{
    BoundedAuthoringSlice, BoundedNodeGeometryInput, ProjectionSeverity,
    project_bounded_with_membership_stats,
};
use pub_model::{Affine2D, CanonicalId, LengthEmu, NodeId, RectEmu, Story, StoryFrame, StoryId};
use serde::Serialize;
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
enum MatrixCase {
    Present,
    MissingStory,
    MissingFrame,
}

#[derive(Debug, Serialize)]
struct BenchmarkRow {
    size: usize,
    case: MatrixCase,
    elapsed_ns: u128,
    story_membership_comparisons: u64,
    frame_membership_comparisons: u64,
    total_membership_comparisons: u64,
    binary_comparison_bound: u64,
    legacy_linear_visits: u64,
    additional_index_bytes: u64,
    diagnostic_count: usize,
    projection_sha256: String,
    dominant_projection_cost_after_fix: &'static str,
}

#[derive(Debug, Serialize)]
struct BenchmarkReceipt {
    schema_version: &'static str,
    architecture_decision_allowed: bool,
    algorithm: &'static str,
    rows: Vec<BenchmarkRow>,
}

fn canonical_id(namespace: u64, value: u64) -> CanonicalId {
    let mut bytes = [0u8; 16];
    bytes[..8].copy_from_slice(&namespace.to_be_bytes());
    bytes[8..].copy_from_slice(&value.to_be_bytes());
    CanonicalId::from_bytes(bytes)
}

fn story_id(value: u64) -> StoryId {
    StoryId::from_canonical(canonical_id(1, value))
}

fn missing_story_id(value: u64) -> StoryId {
    StoryId::from_canonical(canonical_id(3, value))
}

fn node_id(value: u64) -> NodeId {
    NodeId::from_canonical(canonical_id(2, value))
}

fn missing_node_id(value: u64) -> NodeId {
    NodeId::from_canonical(canonical_id(4, value))
}

fn input(size: usize, case: MatrixCase) -> BoundedAuthoringSlice {
    let mut stories = Vec::with_capacity(size);
    let mut node_geometry = Vec::with_capacity(size);
    let mut story_frames = Vec::with_capacity(size);
    let parent_origin = canonical_id(9, 1);

    for index in 0..size {
        let value = u64::try_from(index + 1).expect("benchmark index fits u64");
        let story = story_id(value);
        let node = node_id(value);

        stories.push(Story {
            id: story,
            text: String::new(),
            paragraphs: Vec::new(),
            runs: Vec::new(),
            fields: Vec::new(),
            hyperlinks: Vec::new(),
            source_refs: Vec::new(),
        });
        node_geometry.push(BoundedNodeGeometryInput {
            node_id: node,
            parent_origin,
            bounds: RectEmu::new(
                LengthEmu::new(i64::try_from(index).expect("index fits i64")),
                LengthEmu::new(0),
                LengthEmu::new(1),
                LengthEmu::new(1),
            ),
            transform: Affine2D::identity(),
        });

        story_frames.push(StoryFrame {
            story_id: match case {
                MatrixCase::Present | MatrixCase::MissingFrame => story,
                MatrixCase::MissingStory => missing_story_id(value),
            },
            frame_id: match case {
                MatrixCase::Present | MatrixCase::MissingStory => node,
                MatrixCase::MissingFrame => missing_node_id(value),
            },
            ordinal: 0,
            previous: None,
            next: None,
        });
    }

    BoundedAuthoringSlice {
        pages: Vec::new(),
        node_geometry,
        stories,
        story_frames,
        tables: Vec::new(),
        guides: Vec::new(),
        unknown_layout_state: Vec::new(),
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn triangular(size: u64) -> u64 {
    size.saturating_mul(size.saturating_add(1)) / 2
}

fn legacy_linear_visits(size: usize, case: MatrixCase) -> u64 {
    let size = u64::try_from(size).expect("benchmark size fits u64");
    match case {
        MatrixCase::Present => triangular(size).saturating_mul(2),
        MatrixCase::MissingStory => size.saturating_mul(size).saturating_add(triangular(size)),
        MatrixCase::MissingFrame => triangular(size).saturating_add(size.saturating_mul(size)),
    }
}

fn ceil_log2(value: usize) -> u64 {
    if value <= 1 {
        return 0;
    }
    u64::from(usize::BITS - (value - 1).leading_zeros())
}

fn expected_diagnostics(size: usize, case: MatrixCase) -> usize {
    match case {
        MatrixCase::Present => 0,
        MatrixCase::MissingStory | MatrixCase::MissingFrame => size,
    }
}

#[test]
#[ignore = "hosted performance receipt; run explicitly from owner workflow"]
fn projection_membership_benchmark_matrix() {
    let mut rows = Vec::new();

    for size in [1_000usize, 10_000, 50_000] {
        for case in [
            MatrixCase::Present,
            MatrixCase::MissingStory,
            MatrixCase::MissingFrame,
        ] {
            let start = Instant::now();
            let (projection, stats) = project_bounded_with_membership_stats(input(size, case));
            let elapsed_ns = start.elapsed().as_nanos();

            let diagnostics = projection
                .diagnostics
                .iter()
                .filter(|diagnostic| {
                    diagnostic.severity == ProjectionSeverity::Error
                        && matches!(
                            diagnostic.code.as_str(),
                            "missing_story_content" | "missing_frame_geometry"
                        )
                })
                .count();
            assert_eq!(diagnostics, expected_diagnostics(size, case));
            assert_eq!(stats.additional_index_bytes, 0);

            let per_lookup = ceil_log2(size).saturating_add(1);
            let binary_bound = u64::try_from(size)
                .expect("size fits u64")
                .saturating_mul(per_lookup)
                .saturating_mul(2);
            assert!(
                stats.total_membership_comparisons() <= binary_bound,
                "{size:?}/{case:?}: {} > {binary_bound}",
                stats.total_membership_comparisons()
            );

            let projection_bytes =
                serde_json::to_vec(&projection).expect("serialize benchmark projection");
            let projection_sha256 = sha256_hex(&projection_bytes);

            rows.push(BenchmarkRow {
                size,
                case,
                elapsed_ns,
                story_membership_comparisons: stats.story_membership_comparisons,
                frame_membership_comparisons: stats.frame_membership_comparisons,
                total_membership_comparisons: stats.total_membership_comparisons(),
                binary_comparison_bound: binary_bound,
                legacy_linear_visits: legacy_linear_visits(size, case),
                additional_index_bytes: stats.additional_index_bytes,
                diagnostic_count: diagnostics,
                projection_sha256,
                dominant_projection_cost_after_fix: "normalization_sort_and_output_materialization",
            });
        }
    }

    let receipt = BenchmarkReceipt {
        schema_version: "chaptera.layout-projection-index-benchmark.v1",
        architecture_decision_allowed: false,
        algorithm: "binary_search_over_existing_sorted_story_and_node_vectors",
        rows,
    };
    let json = serde_json::to_vec_pretty(&receipt).expect("serialize benchmark receipt");

    if let Some(path) = env::var_os("LAYOUT_PROJECTION_INDEX_RECEIPT") {
        let path = Path::new(&path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create benchmark receipt directory");
        }
        fs::write(path, &json).expect("write benchmark receipt");
    }

    println!(
        "{}",
        String::from_utf8(json).expect("receipt is UTF-8 JSON")
    );
}
