use anyhow::{Context, Result};
use chaptera_mobile_reader_core::MobileReaderDocumentV1;
use std::{env, fs};

fn main() -> Result<()> {
    let paths = env::args().skip(1).collect::<Vec<_>>();
    if paths.is_empty() {
        anyhow::bail!("usage: mobile-reader-image-probe <pub> [<pub> ...]");
    }

    for path in paths {
        let bytes = fs::read(&path).with_context(|| format!("read {path}"))?;
        let document =
            MobileReaderDocumentV1::open_default(&bytes).with_context(|| format!("open {path}"))?;

        let mut image_refs = 0usize;
        let mut exact_image_refs = 0usize;
        let mut first_mime = None::<String>;

        for page_index in 0..document.page_count() {
            let plan = document
                .page_render_plan(page_index)
                .with_context(|| format!("render plan {path} page {page_index}"))?;
            for node in plan.nodes {
                let Some(image) = node.image else { continue };
                image_refs += 1;
                if first_mime.is_none() {
                    first_mime = Some(image.mime.clone());
                }
                if document
                    .image_resource_bytes(image.resource_id)
                    .is_some_and(|encoded| !encoded.is_empty())
                {
                    exact_image_refs += 1;
                }
            }
        }

        println!(
            "MOBILE_IMAGE_PROBE\tpath={}\tbytes={}\tpages={}\timage_refs={}\texact_image_refs={}\tfirst_mime={}",
            path,
            bytes.len(),
            document.page_count(),
            image_refs,
            exact_image_refs,
            first_mime.as_deref().unwrap_or("none")
        );
    }

    Ok(())
}
