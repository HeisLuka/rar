use chaptera_image_decode_contract::{DecodeLimitsV1, DecodePolicyV1, decode_image_v1, receipt_v1};
use png::{BitDepth, ColorType, Encoder};
use sha2::{Digest, Sha256};

fn main() {
    let mut encoded = Vec::new();
    {
        let mut encoder = Encoder::new(&mut encoded, 2, 1);
        encoder.set_color(ColorType::Rgba);
        encoder.set_depth(BitDepth::Eight);
        let mut writer = encoder.write_header().unwrap();
        writer
            .write_image_data(&[255, 0, 0, 128, 0, 255, 0, 255])
            .unwrap();
    }
    let hash = format!("{:x}", Sha256::digest(&encoded));
    let decoded = decode_image_v1(
        &encoded,
        "image/png",
        &hash,
        "color-disposition:synthetic-srgb",
        &DecodePolicyV1::reference(),
        &DecodeLimitsV1::default(),
    )
    .unwrap();
    println!(
        "{}",
        serde_json::to_string_pretty(&receipt_v1(&decoded)).unwrap()
    );
}
