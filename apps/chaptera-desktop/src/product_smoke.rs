use serde_json::{Value, json};

const PROTOCOL_VERSION: &str = "chaptera.desktop-source-free-smoke-observation.v1";

pub fn run() -> Value {
    let reader_only = cfg!(feature = "reader-only");
    json!({
        "protocol_version": PROTOCOL_VERSION,
        "product": if reader_only { "Chaptera PUB Reader" } else { "Chaptera Editor" },
        "chaptera_version": env!("CARGO_PKG_VERSION"),
        "reader_only": reader_only,
        "editor_controls_enabled": !reader_only,
        "native_save_pub_claimed": false,
        "runtime": {
            "checkout_required": false,
            "cargo_required": false,
            "vendor_path_required": false,
            "external_runtime_assets_required": false
        }
    })
}
