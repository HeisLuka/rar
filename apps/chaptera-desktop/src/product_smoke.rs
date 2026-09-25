use serde_json::{Value, json};
use std::env;

// The clean job supplies independently measured executable identity through the
// environment; this GUI binary reports product/runtime identity and never trusts
// a repository-relative path or bundled source tree.
const PROTOCOL_VERSION: &str = "chaptera.desktop-source-free-smoke.v1";

fn bound_binary_identity() -> Result<(String, u64), String> {
    let sha256 = env::var("CHAPTERA_PRODUCT_SMOKE_BINARY_SHA256")
        .map_err(|_| "CHAPTERA_PRODUCT_SMOKE_BINARY_SHA256 is required".to_owned())?
        .to_ascii_lowercase();
    if sha256.len() != 64 || !sha256.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("product smoke binary SHA-256 must be 64 hexadecimal characters".to_owned());
    }

    let byte_len = env::var("CHAPTERA_PRODUCT_SMOKE_BINARY_BYTE_LEN")
        .map_err(|_| "CHAPTERA_PRODUCT_SMOKE_BINARY_BYTE_LEN is required".to_owned())?
        .parse::<u64>()
        .map_err(|_| "product smoke binary byte length must be an unsigned integer".to_owned())?;
    if byte_len == 0 {
        return Err("product smoke binary byte length must be non-zero".to_owned());
    }

    Ok((sha256, byte_len))
}

pub fn run() -> Result<Value, String> {
    let (binary_sha256, binary_byte_len) = bound_binary_identity()?;
    let reader_only = cfg!(feature = "reader-only");

    Ok(json!({
        "protocol_version": PROTOCOL_VERSION,
        "product": if reader_only { "Chaptera PUB Reader" } else { "Chaptera Editor" },
        "reader_only": reader_only,
        "editor_controls_enabled": !reader_only,
        "native_save_pub_claimed": false,
        "binary_sha256": binary_sha256,
        "binary_byte_len": binary_byte_len,
        "runtime": {
            "checkout_required": false,
            "cargo_required": false,
            "vendor_path_required": false,
            "external_runtime_assets_required": false
        }
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protocol_is_stable() {
        assert_eq!(PROTOCOL_VERSION, "chaptera.desktop-source-free-smoke.v1");
    }
}
