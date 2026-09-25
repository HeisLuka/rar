mod overset;

use anyhow::{Context, Result};
use pub_editor::{EditorEditableTarget, EditorProject};
use pub_model::{Sha256Digest, to_cdm_debug_json_v0_1};
use std::{env, fs, io::Cursor, path::Path};

const SAMPLE_HASH: &str = "6a825ba26ba35d6e885acdc62e859591ed37cb0ff7480b554b9cb362b644dfcf";

fn pinned_hash() -> Sha256Digest {
    SAMPLE_HASH
        .parse()
        .expect("pinned SampleNewsletter SHA-256")
}

fn emit_viewer(path: &str) -> Result<()> {
    let bytes = fs::read(path).context("read pinned PUB fixture")?;
    let visual = pub_viewer::open_mature_0x2c_geometry(
        &bytes,
        pub_viewer::viewer_geometry_environment_v0_1(),
    )
    .context("open mature-0x2c Viewer geometry")?;
    print!("{}", serde_json::to_string(&visual)?);
    Ok(())
}

fn emit_resolved_graph(path: &str) -> Result<()> {
    let bytes = fs::read(path).context("read pinned PUB fixture")?;
    let source =
        pub_reader::build_mature_0x2c_source_graph(Cursor::new(bytes.as_slice()), pinned_hash())
            .context("build mature-0x2c SourceGraph")?;
    let resolved =
        pub_reader::resolve_pub_source_graph(&source.graph).context("resolve PUB SourceGraph")?;
    let canonical =
        to_cdm_debug_json_v0_1(&resolved.graph).context("serialize canonical resolved graph")?;
    std::io::Write::write_all(&mut std::io::stdout(), &canonical)?;
    Ok(())
}

fn editable_target(value: &str) -> Result<EditorEditableTarget> {
    match value {
        "idml" => Ok(EditorEditableTarget::Idml),
        "odg" => Ok(EditorEditableTarget::Odg),
        other => anyhow::bail!("unsupported editable target {other:?}"),
    }
}

fn emit_editable_export(
    fixture: &str,
    project_path: &str,
    target_name: &str,
    output_path: &str,
    report_path: &str,
) -> Result<()> {
    let bytes = fs::read(fixture).context("read pinned PUB fixture")?;
    let project: EditorProject =
        serde_json::from_slice(&fs::read(project_path).context("read canonical EditorProject")?)
            .context("parse canonical EditorProject")?;
    let target = editable_target(target_name)?;
    let mut session =
        pub_editor::open_mature_0x2c_editor(&bytes, pinned_hash()).context("open editor")?;
    session
        .apply_project(&project)
        .context("replay canonical EditorProject")?;
    let preview = session
        .preview_editable_export(target, "SampleNewsletter.pub")
        .context("preview editable export")?;
    let export = session
        .export_editable(target, "SampleNewsletter.pub")
        .context("serialize editable export")?;
    if preview.report != export.report {
        anyhow::bail!("preview/export report mismatch");
    }
    fs::write(Path::new(output_path), &export.bytes).context("write editable export")?;
    fs::write(
        Path::new(report_path),
        serde_json::to_vec_pretty(&export.report).context("serialize export report")?,
    )
    .context("write export report")?;
    print!(
        "{}",
        serde_json::json!({
            "target": target_name,
            "project": project,
            "report": export.report,
            "byte_len": export.bytes.len(),
        })
    );
    Ok(())
}

fn main() -> Result<()> {
    let mut args = env::args().skip(1);
    let first = args
        .next()
        .context("fixture path or mode argument missing")?;
    if first == "authoring-overset" {
        if args.next().is_some() {
            anyhow::bail!("unexpected extra arguments");
        }
        return overset::run();
    }
    if first == "resolved-graph" {
        let path = args.next().context("fixture path argument missing")?;
        if args.next().is_some() {
            anyhow::bail!("unexpected extra arguments");
        }
        return emit_resolved_graph(&path);
    }
    if first == "editable-export" {
        let fixture = args.next().context("fixture path missing")?;
        let project = args.next().context("project path missing")?;
        let target = args.next().context("target missing")?;
        let output = args.next().context("output path missing")?;
        let report = args.next().context("report path missing")?;
        if args.next().is_some() {
            anyhow::bail!("unexpected extra arguments");
        }
        return emit_editable_export(&fixture, &project, &target, &output, &report);
    }
    if args.next().is_some() {
        anyhow::bail!("unexpected extra arguments");
    }
    emit_viewer(&first)
}
