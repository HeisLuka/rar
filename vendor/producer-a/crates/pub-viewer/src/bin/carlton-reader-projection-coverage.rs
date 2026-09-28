use anyhow::{Context, Result};
use pub_model::{CanonicalId, NodeId, PageId};
use pub_reader::{derive_pub_node_id, derive_pub_page_id};
use pub_viewer::{open_mature_0x2c_geometry, viewer_geometry_environment_v0_1};
use serde::Serialize;
use std::{collections::BTreeMap, env, fs, path::PathBuf};

const MARCH_SHA256: &str =
    "bf9cda0f632b5820ab9dbdbe1b838b2a988b2f3fdd69253c22b4fc3aef9f11c3";

#[derive(Debug, Serialize)]
struct OriginCoverage {
    seq_num: u32,
    expected_target_pages: Vec<u32>,
    observed_target_pages: Vec<String>,
    occurrence_count: usize,
}

#[derive(Debug, Serialize)]
struct CarltonReaderProjectionCoverageReceipt {
    schema: String,
    source_sha256: String,
    viewer_page_count: usize,
    scene_surface_count: usize,
    family_profile_applied: bool,
    target_frame_controls: Vec<OriginCoverage>,
    inherited_master_shape: OriginCoverage,
    visible_cmo_carriers: Vec<OriginCoverage>,
    diagnostic_codes: Vec<String>,
}

fn observed_pages_for_origin(
    visual: &pub_viewer::ViewerGeometryDocument,
    origin: NodeId,
) -> Vec<String> {
    let mut pages = visual
        .scene
        .nodes
        .iter()
        .filter(|node| node.origin == origin)
        .map(|node| node.parent_origin.to_string())
        .collect::<Vec<_>>();
    pages.sort();
    pages.dedup();
    pages
}

fn expected_page_ids(
    source_hash: &pub_model::Sha256Digest,
    page_seq_nums: &[u32],
) -> Result<Vec<PageId>> {
    page_seq_nums
        .iter()
        .map(|seq| derive_pub_page_id(source_hash, *seq))
        .collect()
}

fn coverage(
    visual: &pub_viewer::ViewerGeometryDocument,
    source_hash: &pub_model::Sha256Digest,
    seq_num: u32,
    expected_target_pages: &[u32],
) -> Result<OriginCoverage> {
    let origin = derive_pub_node_id(source_hash, seq_num)?;
    let expected_ids = expected_page_ids(source_hash, expected_target_pages)?;
    let observed_target_pages = observed_pages_for_origin(visual, origin);
    let occurrence_count = visual
        .scene
        .nodes
        .iter()
        .filter(|node| node.origin == origin)
        .count();

    // Keep the canonical ids grounded in this receipt even though acceptance
    // below compares page seqNums. This also makes a future projection-instance
    // implementation independently auditable without source bytes.
    let expected_canonical = expected_ids
        .iter()
        .map(|page| page.as_canonical().to_string())
        .collect::<Vec<_>>();
    let observed_set = observed_target_pages
        .iter()
        .cloned()
        .collect::<std::collections::BTreeSet<_>>();
    let _expected_presence = expected_canonical
        .iter()
        .map(|id| (id, observed_set.contains(id)))
        .collect::<BTreeMap<_, _>>();

    Ok(OriginCoverage {
        seq_num,
        expected_target_pages: expected_target_pages.to_vec(),
        observed_target_pages,
        occurrence_count,
    })
}

fn main() -> Result<()> {
    let mut args = env::args_os().skip(1);
    let source = PathBuf::from(
        args.next()
            .context("usage: carlton-reader-projection-coverage SOURCE.pub OUTPUT.json")?,
    );
    let output = PathBuf::from(
        args.next()
            .context("usage: carlton-reader-projection-coverage SOURCE.pub OUTPUT.json")?,
    );
    if args.next().is_some() {
        anyhow::bail!("usage: carlton-reader-projection-coverage SOURCE.pub OUTPUT.json");
    }

    let bytes = fs::read(&source).with_context(|| format!("read {}", source.display()))?;
    let visual = open_mature_0x2c_geometry(&bytes, viewer_geometry_environment_v0_1())
        .context("open exact Carlton March through current product Viewer")?;
    let source_hash = visual.document.source.source_hash;
    let source_sha256 = source_hash.to_string();
    if source_sha256 != MARCH_SHA256 {
        anyhow::bail!("unexpected Carlton March source SHA-256: {source_sha256}");
    }

    let diagnostic_codes = visual
        .document
        .diagnostics
        .iter()
        .map(|diagnostic| diagnostic.code.clone())
        .collect::<Vec<_>>();

    let receipt = CarltonReaderProjectionCoverageReceipt {
        schema: "chaptera.carlton-reader-projection-coverage.v1".to_owned(),
        source_sha256,
        viewer_page_count: visual.document.pages.len(),
        scene_surface_count: visual.scene.surfaces.len(),
        family_profile_applied: diagnostic_codes
            .iter()
            .any(|code| code == "viewer.page_projection.family_profile_applied"),
        target_frame_controls: vec![
            coverage(&visual, &source_hash, 437, &[266])?,
            coverage(&visual, &source_hash, 402, &[361])?,
            coverage(&visual, &source_hash, 435, &[361])?,
            coverage(&visual, &source_hash, 369, &[406])?,
        ],
        inherited_master_shape: coverage(&visual, &source_hash, 380, &[266, 361, 406])?,
        visible_cmo_carriers: vec![
            coverage(&visual, &source_hash, 319, &[266])?,
            coverage(&visual, &source_hash, 343, &[361])?,
            coverage(&visual, &source_hash, 344, &[361])?,
            coverage(&visual, &source_hash, 441, &[406])?,
        ],
        diagnostic_codes,
    };

    fs::write(
        &output,
        serde_json::to_vec_pretty(&receipt).context("serialize projection coverage receipt")?,
    )
    .with_context(|| format!("write {}", output.display()))?;

    println!(
        "CARLTON READER PROJECTION COVERAGE viewer={}/{} target_frames={} master={} cmo={:?}",
        receipt.viewer_page_count,
        receipt.scene_surface_count,
        receipt
            .target_frame_controls
            .iter()
            .map(|item| item.occurrence_count)
            .sum::<usize>(),
        receipt.inherited_master_shape.occurrence_count,
        receipt
            .visible_cmo_carriers
            .iter()
            .map(|item| item.occurrence_count)
            .collect::<Vec<_>>()
    );

    Ok(())
}
