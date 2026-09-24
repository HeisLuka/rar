use jpeg_decoder::{CodingProcess, Decoder as JpegDecoder, PixelFormat};
use png::{BitDepth, ColorType, Decoder as PngDecoder, Transformations};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fmt;
use std::io::Cursor;

pub const DECODE_CONTRACT_VERSION_V1: &str = "chaptera.image-decode-contract.v1";
pub const REFERENCE_DECODER_ID_V1: &str =
    "png:0.18.1/expand+jpeg-decoder:0.3.2/platform-independent";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DecodeLimitsV1 {
    pub max_encoded_bytes: usize,
    pub max_dimension_px: u32,
    pub max_total_pixels: u64,
    pub max_decoded_bytes: usize,
}

impl Default for DecodeLimitsV1 {
    fn default() -> Self {
        Self {
            max_encoded_bytes: 64 * 1024 * 1024,
            max_dimension_px: 100_000,
            max_total_pixels: 100_000_000,
            max_decoded_bytes: 512 * 1024 * 1024,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DecodePolicyV1 {
    pub contract_version: String,
    pub decoder_id: String,
    pub orientation_policy: String,
    pub alpha_policy: String,
    pub precision_policy: String,
    pub source_color_conversion_policy: String,
    pub frame_policy: String,
}

impl DecodePolicyV1 {
    pub fn reference() -> Self {
        Self {
            contract_version: DECODE_CONTRACT_VERSION_V1.into(),
            decoder_id: REFERENCE_DECODER_ID_V1.into(),
            orientation_policy: "preserve_metadata_do_not_apply".into(),
            alpha_policy: "straight_unassociated".into(),
            precision_policy: "expand_low_bit_and_palette_preserve_16bit".into(),
            source_color_conversion_policy:
                "no_icc_transform;jpeg_component_decode_to_pinned_decoder_rgb".into(),
            frame_policy: "single_static_frame_only".into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DecodedImageV1 {
    pub contract_version: String,
    pub resource_sha256: String,
    pub mime_type: String,
    pub codec_family: String,
    pub encoded_width_px: u32,
    pub encoded_height_px: u32,
    pub decoded_width_px: u32,
    pub decoded_height_px: u32,
    pub source_sample_model: String,
    pub decoded_sample_model: String,
    pub source_precision_bits: u8,
    pub decoded_precision_bits: u8,
    pub alpha_present: bool,
    pub alpha_association: String,
    pub orientation_class: String,
    pub exif_orientation: Option<u16>,
    pub orientation_applied: bool,
    pub frame_disposition: String,
    pub coding_process: String,
    pub decoder_id: String,
    pub color_disposition_ref: String,
    pub decoded_byte_count: usize,
    pub sample_digest_sha256: String,
    pub cache_identity_sha256: String,
    #[serde(skip)]
    pub samples: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DecodeReceiptV1 {
    pub contract_version: String,
    pub resource_sha256: String,
    pub mime_type: String,
    pub encoded_dimensions: [u32; 2],
    pub decoded_dimensions: [u32; 2],
    pub source_sample_model: String,
    pub decoded_sample_model: String,
    pub source_precision_bits: u8,
    pub decoded_precision_bits: u8,
    pub alpha_present: bool,
    pub alpha_association: String,
    pub orientation_class: String,
    pub exif_orientation: Option<u16>,
    pub orientation_applied: bool,
    pub frame_disposition: String,
    pub coding_process: String,
    pub decoder_id: String,
    pub color_disposition_ref: String,
    pub decoded_byte_count: usize,
    pub sample_digest_sha256: String,
    pub cache_identity_sha256: String,
    pub unsupported_or_error_code: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodeErrorV1 {
    pub code: &'static str,
    pub detail: String,
}

impl fmt::Display for DecodeErrorV1 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.detail)
    }
}

impl std::error::Error for DecodeErrorV1 {}

fn fail(code: &'static str, detail: impl Into<String>) -> DecodeErrorV1 {
    DecodeErrorV1 {
        code,
        detail: detail.into(),
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn validate_hash(expected: &str, bytes: &[u8]) -> Result<(), DecodeErrorV1> {
    if expected.len() != 64
        || !expected
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
    {
        return Err(fail(
            "invalid_resource_hash",
            "expected SHA-256 must be lowercase hex",
        ));
    }
    if sha256_hex(bytes) != expected {
        return Err(fail(
            "resource_hash_mismatch",
            "exact resource bytes do not match expected SHA-256",
        ));
    }
    Ok(())
}

fn validate_limits(
    encoded_len: usize,
    width: u32,
    height: u32,
    decoded_len: usize,
    limits: &DecodeLimitsV1,
) -> Result<(), DecodeErrorV1> {
    if encoded_len > limits.max_encoded_bytes {
        return Err(fail(
            "encoded_bytes_limit",
            "encoded image exceeds byte limit",
        ));
    }
    if width == 0 || height == 0 {
        return Err(fail(
            "invalid_dimensions",
            "image dimensions must be positive",
        ));
    }
    if width > limits.max_dimension_px || height > limits.max_dimension_px {
        return Err(fail(
            "dimension_limit",
            "image dimension exceeds configured limit",
        ));
    }
    let pixels = u64::from(width)
        .checked_mul(u64::from(height))
        .ok_or_else(|| fail("allocation_overflow", "pixel count overflow"))?;
    if pixels > limits.max_total_pixels {
        return Err(fail(
            "pixel_limit",
            "decoded pixel count exceeds configured limit",
        ));
    }
    if decoded_len > limits.max_decoded_bytes {
        return Err(fail(
            "decoded_bytes_limit",
            "decoded image exceeds byte limit",
        ));
    }
    Ok(())
}

fn png_source_model(color: ColorType) -> &'static str {
    match color {
        ColorType::Grayscale => "png_grayscale",
        ColorType::Rgb => "png_rgb",
        ColorType::Indexed => "png_indexed",
        ColorType::GrayscaleAlpha => "png_grayscale_alpha",
        ColorType::Rgba => "png_rgba",
    }
}

fn png_decoded_model(color: ColorType) -> &'static str {
    match color {
        ColorType::Grayscale => "gray",
        ColorType::Rgb => "rgb",
        ColorType::Indexed => "indexed",
        ColorType::GrayscaleAlpha => "gray_alpha",
        ColorType::Rgba => "rgba",
    }
}

fn bit_depth_bits(depth: BitDepth) -> u8 {
    match depth {
        BitDepth::One => 1,
        BitDepth::Two => 2,
        BitDepth::Four => 4,
        BitDepth::Eight => 8,
        BitDepth::Sixteen => 16,
    }
}

fn cache_identity(
    resource_sha256: &str,
    mime_type: &str,
    policy: &DecodePolicyV1,
    color_disposition_ref: &str,
    frame_disposition: &str,
) -> String {
    let value = serde_json::json!({
        "resource_sha256": resource_sha256,
        "mime_type": mime_type,
        "contract_version": policy.contract_version,
        "decoder_id": policy.decoder_id,
        "orientation_policy": policy.orientation_policy,
        "alpha_policy": policy.alpha_policy,
        "precision_policy": policy.precision_policy,
        "source_color_conversion_policy": policy.source_color_conversion_policy,
        "frame_policy": policy.frame_policy,
        "color_disposition_ref": color_disposition_ref,
        "frame_disposition": frame_disposition,
    });
    sha256_hex(&serde_json::to_vec(&value).expect("cache identity JSON"))
}

fn decode_png(
    bytes: &[u8],
    resource_sha256: &str,
    color_disposition_ref: &str,
    policy: &DecodePolicyV1,
    limits: &DecodeLimitsV1,
) -> Result<DecodedImageV1, DecodeErrorV1> {
    if bytes.len() > limits.max_encoded_bytes {
        return Err(fail(
            "encoded_bytes_limit",
            "encoded PNG exceeds byte limit",
        ));
    }
    let mut decoder = PngDecoder::new(Cursor::new(bytes));
    decoder.set_transformations(Transformations::EXPAND);
    let mut reader = decoder
        .read_info()
        .map_err(|e| fail("png_decode_error", e.to_string()))?;
    let source_color = reader.info().color_type;
    let source_depth = reader.info().bit_depth;
    if reader.info().animation_control.is_some() {
        return Err(fail(
            "animated_or_multiframe_unsupported",
            "APNG is not admitted by V1",
        ));
    }
    let width = reader.info().width;
    let height = reader.info().height;
    validate_limits(bytes.len(), width, height, 0, limits)?;

    let output_size = reader.output_buffer_size().ok_or_else(|| {
        fail(
            "allocation_overflow",
            "PNG output buffer size is not representable",
        )
    })?;
    if output_size > limits.max_decoded_bytes {
        return Err(fail(
            "decoded_bytes_limit",
            "PNG output buffer exceeds configured limit",
        ));
    }
    let mut buffer = vec![0_u8; output_size];
    let output = reader
        .next_frame(&mut buffer)
        .map_err(|e| fail("png_decode_error", e.to_string()))?;
    let samples = buffer[..output.buffer_size()].to_vec();
    validate_limits(bytes.len(), width, height, samples.len(), limits)?;

    let decoded_depth = bit_depth_bits(output.bit_depth);
    let decoded_model = png_decoded_model(output.color_type).to_string();
    let alpha_present = matches!(
        output.color_type,
        ColorType::GrayscaleAlpha | ColorType::Rgba
    );
    let frame_disposition = "single_static".to_string();

    Ok(DecodedImageV1 {
        contract_version: policy.contract_version.clone(),
        resource_sha256: resource_sha256.into(),
        mime_type: "image/png".into(),
        codec_family: "png".into(),
        encoded_width_px: width,
        encoded_height_px: height,
        decoded_width_px: width,
        decoded_height_px: height,
        source_sample_model: png_source_model(source_color).into(),
        decoded_sample_model: decoded_model,
        source_precision_bits: bit_depth_bits(source_depth),
        decoded_precision_bits: decoded_depth,
        alpha_present,
        alpha_association: if alpha_present {
            "straight_unassociated"
        } else {
            "none"
        }
        .into(),
        orientation_class: "normal".into(),
        exif_orientation: None,
        orientation_applied: false,
        frame_disposition: frame_disposition.clone(),
        coding_process: "lossless_png".into(),
        decoder_id: policy.decoder_id.clone(),
        color_disposition_ref: color_disposition_ref.into(),
        decoded_byte_count: samples.len(),
        sample_digest_sha256: sha256_hex(&samples),
        cache_identity_sha256: cache_identity(
            resource_sha256,
            "image/png",
            policy,
            color_disposition_ref,
            &frame_disposition,
        ),
        samples,
    })
}

fn read_u16(bytes: &[u8], offset: usize, little: bool) -> Option<u16> {
    let chunk = bytes.get(offset..offset + 2)?;
    Some(if little {
        u16::from_le_bytes([chunk[0], chunk[1]])
    } else {
        u16::from_be_bytes([chunk[0], chunk[1]])
    })
}

fn read_u32(bytes: &[u8], offset: usize, little: bool) -> Option<u32> {
    let chunk = bytes.get(offset..offset + 4)?;
    Some(if little {
        u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]])
    } else {
        u32::from_be_bytes([chunk[0], chunk[1], chunk[2], chunk[3]])
    })
}

fn parse_exif_orientation(tiff: &[u8]) -> Result<Option<u16>, DecodeErrorV1> {
    if tiff.is_empty() {
        return Ok(None);
    }
    if tiff.len() < 8 {
        return Err(fail(
            "invalid_exif_orientation",
            "EXIF TIFF header is truncated",
        ));
    }
    let little = match &tiff[0..2] {
        b"II" => true,
        b"MM" => false,
        _ => {
            return Err(fail(
                "invalid_exif_orientation",
                "EXIF byte order is invalid",
            ));
        }
    };
    if read_u16(tiff, 2, little) != Some(42) {
        return Err(fail(
            "invalid_exif_orientation",
            "EXIF TIFF magic is invalid",
        ));
    }
    let ifd0 = read_u32(tiff, 4, little)
        .ok_or_else(|| fail("invalid_exif_orientation", "EXIF IFD0 offset is truncated"))?
        as usize;
    let count = read_u16(tiff, ifd0, little)
        .ok_or_else(|| fail("invalid_exif_orientation", "EXIF IFD0 count is truncated"))?
        as usize;
    for index in 0..count {
        let entry = ifd0
            .checked_add(2)
            .and_then(|v| v.checked_add(index.saturating_mul(12)))
            .ok_or_else(|| fail("invalid_exif_orientation", "EXIF entry offset overflow"))?;
        let tag = read_u16(tiff, entry, little)
            .ok_or_else(|| fail("invalid_exif_orientation", "EXIF entry is truncated"))?;
        if tag != 0x0112 {
            continue;
        }
        let ty = read_u16(tiff, entry + 2, little).unwrap_or(0);
        let n = read_u32(tiff, entry + 4, little).unwrap_or(0);
        if ty != 3 || n != 1 {
            return Err(fail(
                "invalid_exif_orientation",
                "EXIF Orientation entry has unexpected type/count",
            ));
        }
        let value = read_u16(tiff, entry + 8, little).ok_or_else(|| {
            fail(
                "invalid_exif_orientation",
                "EXIF Orientation value is truncated",
            )
        })?;
        if !(1..=8).contains(&value) {
            return Err(fail(
                "invalid_exif_orientation",
                "EXIF Orientation must be 1..8",
            ));
        }
        return Ok(Some(value));
    }
    Ok(None)
}

fn jpeg_coding(process: CodingProcess) -> &'static str {
    match process {
        CodingProcess::DctSequential => "jpeg_dct_sequential",
        CodingProcess::DctProgressive => "jpeg_dct_progressive",
        CodingProcess::Lossless => "jpeg_lossless",
    }
}

fn decode_jpeg(
    bytes: &[u8],
    resource_sha256: &str,
    color_disposition_ref: &str,
    policy: &DecodePolicyV1,
    limits: &DecodeLimitsV1,
) -> Result<DecodedImageV1, DecodeErrorV1> {
    if bytes.len() > limits.max_encoded_bytes {
        return Err(fail(
            "encoded_bytes_limit",
            "encoded JPEG exceeds byte limit",
        ));
    }
    let mut decoder = JpegDecoder::new(Cursor::new(bytes));
    decoder.set_max_decoding_buffer_size(limits.max_decoded_bytes);
    decoder
        .read_info()
        .map_err(|e| fail("jpeg_decode_error", e.to_string()))?;
    let info = decoder
        .info()
        .ok_or_else(|| fail("jpeg_decode_error", "JPEG decoder returned no image info"))?;
    let width = u32::from(info.width);
    let height = u32::from(info.height);
    validate_limits(bytes.len(), width, height, 0, limits)?;

    let (source_model, decoded_model, source_precision, decoded_precision, alpha_present) =
        match info.pixel_format {
            PixelFormat::L8 => ("jpeg_grayscale", "gray", 8, 8, false),
            PixelFormat::RGB24 => ("jpeg_ycbcr_or_rgb_display", "rgb", 8, 8, false),
            PixelFormat::L16 => {
                return Err(fail(
                    "unusual_jpeg_precision_unsupported",
                    "16-bit JPEG sample precision is not admitted by V1",
                ));
            }
            PixelFormat::CMYK32 => {
                return Err(fail(
                    "cmyk_or_ycck_unsupported",
                    "CMYK/YCCK JPEG is detected but no V1 display transform is admitted",
                ));
            }
        };
    if info.coding_process == CodingProcess::Lossless {
        return Err(fail(
            "jpeg_lossless_unsupported",
            "lossless JPEG coding is outside V1",
        ));
    }

    let samples = decoder
        .decode()
        .map_err(|e| fail("jpeg_decode_error", e.to_string()))?;
    validate_limits(bytes.len(), width, height, samples.len(), limits)?;
    let exif_orientation = match decoder.exif_data() {
        Some(tiff) => parse_exif_orientation(tiff)?,
        None => None,
    };
    let orientation_class = match exif_orientation {
        None | Some(1) => "normal",
        Some(_) => "metadata_only_non_normal",
    };
    let frame_disposition = "single_static".to_string();

    Ok(DecodedImageV1 {
        contract_version: policy.contract_version.clone(),
        resource_sha256: resource_sha256.into(),
        mime_type: "image/jpeg".into(),
        codec_family: "jpeg".into(),
        encoded_width_px: width,
        encoded_height_px: height,
        decoded_width_px: width,
        decoded_height_px: height,
        source_sample_model: source_model.into(),
        decoded_sample_model: decoded_model.into(),
        source_precision_bits: source_precision,
        decoded_precision_bits: decoded_precision,
        alpha_present,
        alpha_association: "none".into(),
        orientation_class: orientation_class.into(),
        exif_orientation,
        orientation_applied: false,
        frame_disposition: frame_disposition.clone(),
        coding_process: jpeg_coding(info.coding_process).into(),
        decoder_id: policy.decoder_id.clone(),
        color_disposition_ref: color_disposition_ref.into(),
        decoded_byte_count: samples.len(),
        sample_digest_sha256: sha256_hex(&samples),
        cache_identity_sha256: cache_identity(
            resource_sha256,
            "image/jpeg",
            policy,
            color_disposition_ref,
            &frame_disposition,
        ),
        samples,
    })
}

pub fn decode_image_v1(
    bytes: &[u8],
    mime_type: &str,
    expected_sha256: &str,
    color_disposition_ref: &str,
    policy: &DecodePolicyV1,
    limits: &DecodeLimitsV1,
) -> Result<DecodedImageV1, DecodeErrorV1> {
    validate_hash(expected_sha256, bytes)?;
    match mime_type {
        "image/png" => decode_png(
            bytes,
            expected_sha256,
            color_disposition_ref,
            policy,
            limits,
        ),
        "image/jpeg" => decode_jpeg(
            bytes,
            expected_sha256,
            color_disposition_ref,
            policy,
            limits,
        ),
        _ => Err(fail(
            "unsupported_codec",
            "V1 admits only image/png and image/jpeg",
        )),
    }
}

pub fn receipt_v1(decoded: &DecodedImageV1) -> DecodeReceiptV1 {
    DecodeReceiptV1 {
        contract_version: decoded.contract_version.clone(),
        resource_sha256: decoded.resource_sha256.clone(),
        mime_type: decoded.mime_type.clone(),
        encoded_dimensions: [decoded.encoded_width_px, decoded.encoded_height_px],
        decoded_dimensions: [decoded.decoded_width_px, decoded.decoded_height_px],
        source_sample_model: decoded.source_sample_model.clone(),
        decoded_sample_model: decoded.decoded_sample_model.clone(),
        source_precision_bits: decoded.source_precision_bits,
        decoded_precision_bits: decoded.decoded_precision_bits,
        alpha_present: decoded.alpha_present,
        alpha_association: decoded.alpha_association.clone(),
        orientation_class: decoded.orientation_class.clone(),
        exif_orientation: decoded.exif_orientation,
        orientation_applied: decoded.orientation_applied,
        frame_disposition: decoded.frame_disposition.clone(),
        coding_process: decoded.coding_process.clone(),
        decoder_id: decoded.decoder_id.clone(),
        color_disposition_ref: decoded.color_disposition_ref.clone(),
        decoded_byte_count: decoded.decoded_byte_count,
        sample_digest_sha256: decoded.sample_digest_sha256.clone(),
        cache_identity_sha256: decoded.cache_identity_sha256.clone(),
        unsupported_or_error_code: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jpeg_encoder::{ColorType as JpegColorType, Encoder as JpegEncoder};
    use png::{BitDepth, ColorType, Encoder as PngEncoder};

    fn hash(data: &[u8]) -> String {
        sha256_hex(data)
    }

    fn png_fixture(
        color: ColorType,
        depth: BitDepth,
        width: u32,
        height: u32,
        samples: &[u8],
    ) -> Vec<u8> {
        let mut out = Vec::new();
        {
            let mut encoder = PngEncoder::new(&mut out, width, height);
            encoder.set_color(color);
            encoder.set_depth(depth);
            let mut writer = encoder.write_header().unwrap();
            writer.write_image_data(samples).unwrap();
        }
        out
    }

    fn palette_png() -> Vec<u8> {
        let mut out = Vec::new();
        {
            let mut encoder = PngEncoder::new(&mut out, 2, 1);
            encoder.set_color(ColorType::Indexed);
            encoder.set_depth(BitDepth::Eight);
            encoder.set_palette(vec![255, 0, 0, 0, 255, 0]);
            let mut writer = encoder.write_header().unwrap();
            writer.write_image_data(&[0, 1]).unwrap();
        }
        out
    }

    fn jpeg_fixture(
        color: JpegColorType,
        data: &[u8],
        progressive: bool,
        exif: Option<&[u8]>,
    ) -> Vec<u8> {
        let mut out = Vec::new();
        let mut encoder = JpegEncoder::new(&mut out, 95);
        encoder.set_progressive(progressive);
        if let Some(exif) = exif {
            encoder.add_exif_metadata(exif).unwrap();
        }
        encoder.encode(data, 2, 2, color).unwrap();
        out
    }

    fn exif_orientation_tiff(value: u16) -> Vec<u8> {
        let mut tiff = Vec::new();
        tiff.extend_from_slice(b"II");
        tiff.extend_from_slice(&42_u16.to_le_bytes());
        tiff.extend_from_slice(&8_u32.to_le_bytes());
        tiff.extend_from_slice(&1_u16.to_le_bytes());
        tiff.extend_from_slice(&0x0112_u16.to_le_bytes());
        tiff.extend_from_slice(&3_u16.to_le_bytes());
        tiff.extend_from_slice(&1_u32.to_le_bytes());
        tiff.extend_from_slice(&value.to_le_bytes());
        tiff.extend_from_slice(&0_u16.to_le_bytes());
        tiff.extend_from_slice(&0_u32.to_le_bytes());
        tiff
    }

    fn decode(bytes: &[u8], mime: &str) -> DecodedImageV1 {
        decode_image_v1(
            bytes,
            mime,
            &hash(bytes),
            "color-disposition:test",
            &DecodePolicyV1::reference(),
            &DecodeLimitsV1::default(),
        )
        .unwrap()
    }

    #[test]
    fn png_grayscale_rgb_ga_rgba_decode_deterministically() {
        let fixtures = [
            (ColorType::Grayscale, vec![0, 64, 128, 255], "gray"),
            (
                ColorType::Rgb,
                vec![255, 0, 0, 0, 255, 0, 0, 0, 255, 255, 255, 255],
                "rgb",
            ),
            (
                ColorType::GrayscaleAlpha,
                vec![0, 0, 64, 128, 128, 200, 255, 255],
                "gray_alpha",
            ),
            (
                ColorType::Rgba,
                vec![
                    255, 0, 0, 10, 0, 255, 0, 20, 0, 0, 255, 30, 255, 255, 255, 40,
                ],
                "rgba",
            ),
        ];
        for (color, bytes, model) in fixtures {
            let encoded = png_fixture(color, BitDepth::Eight, 2, 2, &bytes);
            let a = decode(&encoded, "image/png");
            let b = decode(&encoded, "image/png");
            assert_eq!(a.decoded_sample_model, model);
            assert_eq!(a.sample_digest_sha256, b.sample_digest_sha256);
            assert_eq!(a.samples, b.samples);
        }
    }

    #[test]
    fn png_palette_and_low_bit_are_expanded_under_named_policy() {
        let palette = decode(&palette_png(), "image/png");
        assert_eq!(palette.source_sample_model, "png_indexed");
        assert_eq!(palette.decoded_sample_model, "rgb");
        assert_eq!(palette.decoded_precision_bits, 8);

        let low = png_fixture(ColorType::Grayscale, BitDepth::One, 2, 1, &[0b1000_0000]);
        let decoded = decode(&low, "image/png");
        assert_eq!(decoded.source_precision_bits, 1);
        assert_eq!(decoded.decoded_precision_bits, 8);
        assert_eq!(decoded.decoded_byte_count, 2);
    }

    #[test]
    fn png_16bit_is_preserved_not_silently_truncated() {
        let encoded = png_fixture(
            ColorType::Rgb,
            BitDepth::Sixteen,
            1,
            1,
            &[0x12, 0x34, 0x56, 0x78, 0x9a, 0xbc],
        );
        let decoded = decode(&encoded, "image/png");
        assert_eq!(decoded.source_precision_bits, 16);
        assert_eq!(decoded.decoded_precision_bits, 16);
        assert_eq!(decoded.decoded_byte_count, 6);
    }

    #[test]
    fn alpha_is_explicit_and_straight() {
        let encoded = png_fixture(ColorType::Rgba, BitDepth::Eight, 1, 1, &[100, 50, 25, 128]);
        let decoded = decode(&encoded, "image/png");
        assert!(decoded.alpha_present);
        assert_eq!(decoded.alpha_association, "straight_unassociated");
    }

    #[test]
    fn jpeg_grayscale_baseline_and_rgb_progressive_are_admitted() {
        let gray = jpeg_fixture(JpegColorType::Luma, &[0, 64, 128, 255], false, None);
        let gray_decoded = decode(&gray, "image/jpeg");
        assert_eq!(gray_decoded.decoded_sample_model, "gray");
        assert_eq!(gray_decoded.coding_process, "jpeg_dct_sequential");

        let rgb = jpeg_fixture(
            JpegColorType::Rgb,
            &[255, 0, 0, 0, 255, 0, 0, 0, 255, 255, 255, 255],
            true,
            None,
        );
        let rgb_decoded = decode(&rgb, "image/jpeg");
        assert_eq!(rgb_decoded.decoded_sample_model, "rgb");
        assert_eq!(rgb_decoded.coding_process, "jpeg_dct_progressive");
    }

    #[test]
    fn cmyk_jpeg_is_detected_not_silently_treated_as_rgb() {
        let cmyk = jpeg_fixture(
            JpegColorType::Cmyk,
            &[0, 255, 255, 0, 255, 0, 255, 0, 255, 255, 0, 0, 0, 0, 0, 0],
            false,
            None,
        );
        let err = decode_image_v1(
            &cmyk,
            "image/jpeg",
            &hash(&cmyk),
            "color:test",
            &DecodePolicyV1::reference(),
            &DecodeLimitsV1::default(),
        )
        .unwrap_err();
        assert_eq!(err.code, "cmyk_or_ycck_unsupported");
    }

    #[test]
    fn exif_orientation_is_recorded_but_never_applied() {
        let tiff = exif_orientation_tiff(6);
        let rgb = jpeg_fixture(
            JpegColorType::Rgb,
            &[255, 0, 0, 0, 255, 0, 0, 0, 255, 255, 255, 255],
            false,
            Some(&tiff),
        );
        let decoded = decode(&rgb, "image/jpeg");
        assert_eq!(decoded.exif_orientation, Some(6));
        assert_eq!(decoded.orientation_class, "metadata_only_non_normal");
        assert!(!decoded.orientation_applied);
        assert_eq!(
            (decoded.encoded_width_px, decoded.encoded_height_px),
            (2, 2)
        );
        assert_eq!(
            (decoded.decoded_width_px, decoded.decoded_height_px),
            (2, 2)
        );
    }

    #[test]
    fn hostile_limits_and_truncation_fail_closed() {
        let encoded = png_fixture(ColorType::Rgb, BitDepth::Eight, 2, 2, &[0; 12]);
        let limits = DecodeLimitsV1 {
            max_total_pixels: 3,
            ..DecodeLimitsV1::default()
        };
        let err = decode_image_v1(
            &encoded,
            "image/png",
            &hash(&encoded),
            "color:test",
            &DecodePolicyV1::reference(),
            &limits,
        )
        .unwrap_err();
        assert_eq!(err.code, "pixel_limit");

        let truncated = &encoded[..encoded.len() / 2];
        let err = decode_image_v1(
            truncated,
            "image/png",
            &hash(truncated),
            "color:test",
            &DecodePolicyV1::reference(),
            &DecodeLimitsV1::default(),
        )
        .unwrap_err();
        assert_eq!(err.code, "png_decode_error");
    }

    #[test]
    fn trailing_decoder_tolerance_cannot_redefine_exact_resource_identity() {
        let base = png_fixture(ColorType::Rgb, BitDepth::Eight, 1, 1, &[1, 2, 3]);
        let mut tailed = base.clone();
        tailed.extend_from_slice(b"officeart-adjacent-bytes");
        let a = decode(&base, "image/png");
        let b = decode(&tailed, "image/png");
        assert_eq!(a.sample_digest_sha256, b.sample_digest_sha256);
        assert_ne!(a.resource_sha256, b.resource_sha256);
        assert_ne!(a.cache_identity_sha256, b.cache_identity_sha256);
    }

    #[test]
    fn cache_identity_separates_policy_changes() {
        let encoded = png_fixture(ColorType::Rgb, BitDepth::Eight, 1, 1, &[1, 2, 3]);
        let reference = DecodePolicyV1::reference();
        let mut changed = reference.clone();
        changed.alpha_policy = "premultiplied".into();
        let a = decode_image_v1(
            &encoded,
            "image/png",
            &hash(&encoded),
            "color:test",
            &reference,
            &DecodeLimitsV1::default(),
        )
        .unwrap();
        let b = decode_image_v1(
            &encoded,
            "image/png",
            &hash(&encoded),
            "color:test",
            &changed,
            &DecodeLimitsV1::default(),
        )
        .unwrap();
        assert_ne!(a.cache_identity_sha256, b.cache_identity_sha256);
        assert_eq!(a.sample_digest_sha256, b.sample_digest_sha256);
    }

    #[test]
    fn cold_warm_evicted_redecode_converges_to_same_normalized_receipt() {
        let encoded = png_fixture(
            ColorType::Rgba,
            BitDepth::Eight,
            2,
            1,
            &[1, 2, 3, 4, 5, 6, 7, 8],
        );
        let cold = decode(&encoded, "image/png");
        let warm = decode(&encoded, "image/png");
        let evicted = decode(&encoded, "image/png");
        assert_eq!(receipt_v1(&cold), receipt_v1(&warm));
        assert_eq!(receipt_v1(&cold), receipt_v1(&evicted));
    }
}
