use anyhow::{Context, Result};
use pub_model::{CanonicalId, PageId, Sha256Digest};
use pub_viewer::{
    ViewerPresentationSelection, open_mature_0x2c_geometry,
    open_mature_0x2c_geometry_with_presentation, viewer_geometry_environment_v0_1,
};
use serde::{Deserialize, Serialize};
use std::{env, fs, path::PathBuf};

const RECEIPT_SCHEMA: &str = "chaptera.viewer-presentation-selection-receipt.v1";

#[derive(Debug, Deserialize)]
struct AdmittedPresentationManifest {
    profile_id: String,
    source_sha256: Sha256Digest,
    customer_page_ids: Vec<CanonicalId>,
}

#[derive(Debug, Serialize)]
struct ViewerPresentationSelectionReceipt {
    schema: String,
    profile_id: String,
    source_sha256: Sha256Digest,
    generic_viewer_page_count: usize,
    generic_scene_surface_count: usize,
    selected_viewer_page_count: usize,
    selected_scene_surface_count: usize,
    selected_viewer_page_ids: Vec<CanonicalId>,
    selected_scene_surface_ids: Vec<CanonicalId>,
    diagnostic_codes: Vec<String>,
}

fn main() -> Result<()> {
    let mut args = env::args_os().skip(1);
    let source_path = PathBuf::from(
        args.next()
            .context("usage: presentation-selection-receipt SOURCE.pub PROFILE.json OUTPUT.json")?,
    );
    let profile_path = PathBuf::from(
        args.next()
            .context("usage: presentation-selection-receipt SOURCE.pub PROFILE.json OUTPUT.json")?,
    );
    let output_path = PathBuf::from(
        args.next()
            .context("usage: presentation-selection-receipt SOURCE.pub PROFILE.json OUTPUT.json")?,
    );
    if args.next().is_some() {
        anyhow::bail!(
            "usage: presentation-selection-receipt SOURCE.pub PROFILE.json OUTPUT.json"
        );
    }

    let bytes = fs::read(&source_path)
        .with_context(|| format!("read source PUB {}", source_path.display()))?;
    let manifest: AdmittedPresentationManifest = serde_json::from_slice(
        &fs::read(&profile_path)
            .with_context(|| format!("read presentation manifest {}", profile_path.display()))?,
    )
    .context("parse admitted presentation manifest")?;

    let generic = open_mature_0x2c_geometry(&bytes, viewer_geometry_environment_v0_1())
        .context("open generic no-loss Viewer geometry")?;

    let selection = ViewerPresentationSelection {
        profile_id: manifest.profile_id.clone(),
        source_hash: manifest.source_sha256,
        page_ids: manifest
            .customer_page_ids
            .iter()
            .copied()
            .map(PageId::from_canonical)
            .collect(),
    };
    let selected = open_mature_0x2c_geometry_with_presentation(
        &bytes,
        viewer_geometry_environment_v0_1(),
        &selection,
    )
    .context("apply admitted presentation selection")?;

    let receipt = ViewerPresentationSelectionReceipt {
        schema: RECEIPT_SCHEMA.to_owned(),
        profile_id: selection.profile_id,
        source_sha256: selection.source_hash,
        generic_viewer_page_count: generic.document.pages.len(),
        generic_scene_surface_count: generic.scene.surfaces.len(),
        selected_viewer_page_count: selected.document.pages.len(),
        selected_scene_surface_count: selected.scene.surfaces.len(),
        selected_viewer_page_ids: selected
            .document
            .pages
            .iter()
            .map(|page| page.id.into_canonical())
            .collect(),
        selected_scene_surface_ids: selected
            .scene
            .surfaces
            .iter()
            .map(|surface| surface.origin.into_canonical())
            .collect(),
        diagnostic_codes: selected
            .document
            .diagnostics
            .iter()
            .map(|diagnostic| diagnostic.code.clone())
            .collect(),
    };

    fs::write(
        &output_path,
        serde_json::to_vec_pretty(&receipt).context("serialize Viewer presentation receipt")?,
    )
    .with_context(|| format!("write receipt {}", output_path.display()))?;

    println!(
        "VIEWER PRESENTATION SELECTION PASS profile={} generic={}/{} selected={}/{}",
        receipt.profile_id,
        receipt.generic_viewer_page_count,
        receipt.generic_scene_surface_count,
        receipt.selected_viewer_page_count,
        receipt.selected_scene_surface_count
    );

    Ok(())
}
