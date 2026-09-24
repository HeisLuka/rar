use anyhow::{Context, Result};
use std::{env, fs};

fn main() -> Result<()> {
    let path = env::args().nth(1).context("fixture path argument missing")?;
    let bytes = fs::read(path).context("read pinned PUB fixture")?;
    let visual = pub_viewer::open_mature_0x2c_geometry(
        &bytes,
        pub_viewer::viewer_geometry_environment_v0_1(),
    )
    .context("open mature-0x2c Viewer geometry")?;
    print!("{}", serde_json::to_string(&visual)?);
    Ok(())
}
