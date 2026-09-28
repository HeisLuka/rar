use anyhow::{Context, Result};
use pub_viewer::{open_mature_0x2c_geometry, viewer_geometry_environment_v0_1};
use serde_json::json;
use std::{env, fs, path::PathBuf};

fn main() -> Result<()> {
    let mut args = env::args_os().skip(1);
    let source = PathBuf::from(
        args.next()
            .context("usage: page-projection-receipt SOURCE.pub OUTPUT.json")?,
    );
    let output = PathBuf::from(
        args.next()
            .context("usage: page-projection-receipt SOURCE.pub OUTPUT.json")?,
    );

    let bytes = fs::read(&source).with_context(|| format!("read {}", source.display()))?;
    let visual = open_mature_0x2c_geometry(&bytes, viewer_geometry_environment_v0_1())
        .context("open PUB through current Viewer page projection")?;

    let inherited_typography_run_count = visual
        .typography_runs
        .iter()
        .filter(|run| run.font_inherited || run.size_inherited)
        .count();

    let receipt = json!({
        "schema": "chaptera.viewer-page-projection-receipt.v2",
        "viewer_page_count": visual.document.pages.len(),
        "viewer_page_indices": visual.document.pages.iter().map(|page| page.index).collect::<Vec<_>>(),
        "scene_surface_count": visual.scene.surfaces.len(),
        "scene_node_count": visual.scene.nodes.len(),
        "typography_run_count": visual.typography_runs.len(),
        "inherited_typography_run_count": inherited_typography_run_count,
        "diagnostic_codes": visual.document.diagnostics.iter().map(|diagnostic| diagnostic.code.clone()).collect::<Vec<_>>(),
    });

    fs::write(
        &output,
        serde_json::to_vec_pretty(&receipt).context("serialize page projection receipt")?,
    )
    .with_context(|| format!("write {}", output.display()))?;
    Ok(())
}
