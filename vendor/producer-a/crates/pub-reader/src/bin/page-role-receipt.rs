use anyhow::{Context, Result};
use pub_reader::analyze_mature_0x2c_page_roles;
use std::{env, fs::File, io::BufWriter};

fn main() -> Result<()> {
    let mut args = env::args_os().skip(1);
    let source = args
        .next()
        .context("usage: page-role-receipt SOURCE.pub [OUTPUT.json]")?;
    let output = args.next();

    let file = File::open(&source).with_context(|| format!("open {:?}", source))?;
    let receipt = analyze_mature_0x2c_page_roles(file)?;

    match output {
        Some(path) => {
            let file = File::create(&path).with_context(|| format!("create {:?}", path))?;
            serde_json::to_writer_pretty(BufWriter::new(file), &receipt)?;
        }
        None => {
            println!("{}", serde_json::to_string_pretty(&receipt)?);
        }
    }

    Ok(())
}
