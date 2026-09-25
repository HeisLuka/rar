use chaptera_image_decode_contract::{DecodeLimitsV1, DecodePolicyV1, decode_image_v1, receipt_v1};
use sha2::{Digest, Sha256};
use std::{env, fs, process};

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() != 4 {
        eprintln!("usage: decode_file_receipt <path> <mime-type> <color-disposition-ref>");
        process::exit(2);
    }
    let bytes = fs::read(&args[1]).unwrap_or_else(|error| {
        eprintln!("could not read {}: {error}", args[1]);
        process::exit(2);
    });
    let hash = format!("{:x}", Sha256::digest(&bytes));
    let decoded = decode_image_v1(
        &bytes,
        &args[2],
        &hash,
        &args[3],
        &DecodePolicyV1::reference(),
        &DecodeLimitsV1::default(),
    )
    .unwrap_or_else(|error| {
        eprintln!("{error}");
        process::exit(1);
    });
    println!("{}", serde_json::to_string_pretty(&receipt_v1(&decoded)).unwrap());
}
