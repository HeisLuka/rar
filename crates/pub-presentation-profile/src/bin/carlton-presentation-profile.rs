use anyhow::{Context, Result};
use pub_presentation_profile::{
    CarltonPresentationProfileInputV1, build_carlton_presentation_manifest_v1,
};
use std::{
    env,
    fs::File,
    io::{BufReader, BufWriter},
};

fn main() -> Result<()> {
    let mut args = env::args_os().skip(1);
    let input_path = args
        .next()
        .context("usage: carlton-presentation-profile INPUT.json [OUTPUT.json]")?;
    let output_path = args.next();

    let input: CarltonPresentationProfileInputV1 = serde_json::from_reader(BufReader::new(
        File::open(&input_path).with_context(|| format!("open {:?}", input_path))?,
    ))
    .context("parse Carlton presentation profile input")?;
    let manifest = build_carlton_presentation_manifest_v1(input)
        .context("build admitted Carlton presentation manifest")?;

    match output_path {
        Some(path) => {
            let file = File::create(&path).with_context(|| format!("create {:?}", path))?;
            serde_json::to_writer_pretty(BufWriter::new(file), &manifest)?;
        }
        None => println!("{}", serde_json::to_string_pretty(&manifest)?),
    }
    Ok(())
}
