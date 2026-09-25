use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{env, fs};

const PROTOCOL_VERSION: &str = "chaptera.desktop-source-free-smoke.v1";

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut encoded = String::with_capacity(64);
    for byte in digest {
        use std::fmt::Write as _;
        write!(&mut encoded, "{byte:02x}").expect("writing lowercase hex into String cannot fail");
    }
    encoded
}

pub fn run() -> Result<Value, String> {
    let executable = env::current_exe()
        .map_err(|error| format!("resolve current executable: {error}"))?;
    let bytes = fs::read(&executable)
        .map_err(|error| format!("read current executable: {error}"))?;
    if bytes.len() < 2 || &bytes[..2] != b"MZ" {
        return Err("current executable is not a Windows PE image".to_owned());
    }

    let reader_only = cfg!(feature = "reader-only");
    Ok(json!({
        "protocol_version": PROTOCOL_VERSION,
        "product": if reader_only { "Chaptera PUB Reader" } else { "Chaptera Editor" },
        "reader_only": reader_only,
        "editor_controls_enabled": !reader_only,
        "native_save_pub_claimed": false,
        "binary_sha256": sha256_hex(&bytes),
        "binary_byte_len": bytes.len(),
        "runtime": {
            "checkout_required": false,
            "cargo_required": false,
            "vendor_path_required": false,
            "external_runtime_assets_required": false
        }
    }))
}
