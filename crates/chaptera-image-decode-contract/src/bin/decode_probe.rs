use chaptera_image_decode_contract::{DecodeLimitsV1, DecodePolicyV1, decode_image_v1, receipt_v1};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{env, fs, process};

fn usage() -> ! {
    eprintln!("usage: decode_probe <path> <image/png|image/jpeg> <color-disposition-ref>");
    process::exit(2);
}

fn main() {
    let mut args = env::args().skip(1);
    let path = args.next().unwrap_or_else(|| usage());
    let mime = args.next().unwrap_or_else(|| usage());
    let color = args.next().unwrap_or_else(|| usage());
    if args.next().is_some() {
        usage();
    }

    let bytes = fs::read(&path).unwrap_or_else(|error| {
        eprintln!("read_error:{error}");
        process::exit(3);
    });
    let hash = format!("{:x}", Sha256::digest(&bytes));
    let decoded = decode_image_v1(
        &bytes,
        &mime,
        &hash,
        &color,
        &DecodePolicyV1::reference(),
        &DecodeLimitsV1::default(),
    )
    .unwrap_or_else(|error| {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "probe_kind": "chaptera.image-decode-reference-probe.v1",
                "error_code": error.code,
                "detail": error.detail,
                "resource_sha256": hash,
                "mime_type": mime,
            }))
            .unwrap()
        );
        process::exit(4);
    });
    let receipt = receipt_v1(&decoded);
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "probe_kind": "chaptera.image-decode-reference-probe.v1",
            "receipt": receipt,
            "samples": decoded.samples,
        }))
        .unwrap()
    );
}
