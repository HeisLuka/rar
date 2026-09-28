use chaptera_image_decode_contract::{
    DecodeLimitsV1, DecodePolicyV1, DecodedImageV1, decode_image_v1,
};
use sha2::{Digest, Sha256};
use std::fmt;

pub const MOBILE_ANDROID_COLOR_DISPOSITION_REF_V1: &str =
    "chaptera.mobile.android.argb8888.v1";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MobileAdmittedImageV1 {
    pub width_px: u32,
    pub height_px: u32,
    pub argb8888: Vec<u32>,
    pub cache_identity_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MobileImageDecodeError {
    pub code: String,
    pub detail: String,
}

impl fmt::Display for MobileImageDecodeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.code, self.detail)
    }
}

impl std::error::Error for MobileImageDecodeError {}

fn exact_sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

pub fn decode_mobile_image_v1(
    bytes: &[u8],
    mime: &str,
) -> Result<MobileAdmittedImageV1, MobileImageDecodeError> {
    decode_mobile_image_with_limits(bytes, mime, &DecodeLimitsV1::default())
}

fn decode_mobile_image_with_limits(
    bytes: &[u8],
    mime: &str,
    limits: &DecodeLimitsV1,
) -> Result<MobileAdmittedImageV1, MobileImageDecodeError> {
    let expected_sha256 = exact_sha256_hex(bytes);
    let decoded = decode_image_v1(
        bytes,
        mime,
        &expected_sha256,
        MOBILE_ANDROID_COLOR_DISPOSITION_REF_V1,
        &DecodePolicyV1::reference(),
        limits,
    )
    .map_err(|error| MobileImageDecodeError {
        code: error.code.to_owned(),
        detail: error.detail,
    })?;
    admit_decoded_image_v1(&decoded)
}

fn admit_decoded_image_v1(
    decoded: &DecodedImageV1,
) -> Result<MobileAdmittedImageV1, MobileImageDecodeError> {
    if decoded.decoded_precision_bits != 8 {
        return Err(MobileImageDecodeError {
            code: "mobile_precision_unsupported".to_owned(),
            detail: format!(
                "Android ARGB8888 adapter admits only 8-bit decoded samples; found {}-bit",
                decoded.decoded_precision_bits
            ),
        });
    }
    if decoded.orientation_class != "normal" || decoded.orientation_applied {
        return Err(MobileImageDecodeError {
            code: "mobile_orientation_unsupported".to_owned(),
            detail: format!(
                "Android image adapter does not silently apply orientation {:?}",
                decoded.orientation_class
            ),
        });
    }

    let width = usize::try_from(decoded.decoded_width_px).map_err(|_| MobileImageDecodeError {
        code: "mobile_dimension_overflow".to_owned(),
        detail: "decoded width does not fit mobile address space".to_owned(),
    })?;
    let height = usize::try_from(decoded.decoded_height_px).map_err(|_| MobileImageDecodeError {
        code: "mobile_dimension_overflow".to_owned(),
        detail: "decoded height does not fit mobile address space".to_owned(),
    })?;
    let pixels = width.checked_mul(height).ok_or_else(|| MobileImageDecodeError {
        code: "mobile_dimension_overflow".to_owned(),
        detail: "decoded pixel count overflows mobile address space".to_owned(),
    })?;

    let channels = match decoded.decoded_sample_model.as_str() {
        "gray" => 1,
        "gray_alpha" => 2,
        "rgb" => 3,
        "rgba" => 4,
        other => {
            return Err(MobileImageDecodeError {
                code: "mobile_sample_model_unsupported".to_owned(),
                detail: format!(
                    "decoded sample model {other:?} is not admitted by the Android adapter"
                ),
            });
        }
    };
    let expected_len = pixels.checked_mul(channels).ok_or_else(|| MobileImageDecodeError {
        code: "mobile_sample_length_overflow".to_owned(),
        detail: "decoded sample length overflows mobile address space".to_owned(),
    })?;
    if decoded.samples.len() != expected_len {
        return Err(MobileImageDecodeError {
            code: "mobile_sample_length_mismatch".to_owned(),
            detail: format!(
                "decoded sample length {} does not match {}x{}x{} = {}",
                decoded.samples.len(),
                width,
                height,
                channels,
                expected_len
            ),
        });
    }

    let mut argb8888 = Vec::with_capacity(pixels);
    match decoded.decoded_sample_model.as_str() {
        "gray" => {
            for &gray in &decoded.samples {
                argb8888.push(
                    (0xff_u32 << 24)
                        | (u32::from(gray) << 16)
                        | (u32::from(gray) << 8)
                        | u32::from(gray),
                );
            }
        }
        "gray_alpha" => {
            for sample in decoded.samples.chunks_exact(2) {
                argb8888.push(
                    (u32::from(sample[1]) << 24)
                        | (u32::from(sample[0]) << 16)
                        | (u32::from(sample[0]) << 8)
                        | u32::from(sample[0]),
                );
            }
        }
        "rgb" => {
            for sample in decoded.samples.chunks_exact(3) {
                argb8888.push(
                    (0xff_u32 << 24)
                        | (u32::from(sample[0]) << 16)
                        | (u32::from(sample[1]) << 8)
                        | u32::from(sample[2]),
                );
            }
        }
        "rgba" => {
            for sample in decoded.samples.chunks_exact(4) {
                argb8888.push(
                    (u32::from(sample[3]) << 24)
                        | (u32::from(sample[0]) << 16)
                        | (u32::from(sample[1]) << 8)
                        | u32::from(sample[2]),
                );
            }
        }
        _ => unreachable!("sample model was admitted above"),
    }

    Ok(MobileAdmittedImageV1 {
        width_px: decoded.decoded_width_px,
        height_px: decoded.decoded_height_px,
        argb8888,
        cache_identity_sha256: decoded.cache_identity_sha256.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use png::{BitDepth, ColorType, Encoder};

    fn png_fixture(width: u32, height: u32, samples: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        {
            let mut encoder = Encoder::new(&mut out, width, height);
            encoder.set_color(ColorType::Rgba);
            encoder.set_depth(BitDepth::Eight);
            let mut writer = encoder.write_header().expect("PNG header");
            writer.write_image_data(samples).expect("PNG samples");
        }
        out
    }

    fn decoded(
        model: &str,
        precision: u8,
        orientation: &str,
        samples: Vec<u8>,
    ) -> DecodedImageV1 {
        DecodedImageV1 {
            contract_version: "test".to_owned(),
            resource_sha256: "ab".repeat(32),
            mime_type: "image/png".to_owned(),
            codec_family: "png".to_owned(),
            encoded_width_px: 2,
            encoded_height_px: 1,
            decoded_width_px: 2,
            decoded_height_px: 1,
            source_sample_model: model.to_owned(),
            decoded_sample_model: model.to_owned(),
            source_precision_bits: precision,
            decoded_precision_bits: precision,
            alpha_present: matches!(model, "gray_alpha" | "rgba"),
            alpha_association: if matches!(model, "gray_alpha" | "rgba") {
                "straight_unassociated"
            } else {
                "none"
            }
            .to_owned(),
            orientation_class: orientation.to_owned(),
            exif_orientation: None,
            orientation_applied: false,
            frame_disposition: "single_static".to_owned(),
            coding_process: "test".to_owned(),
            decoder_id: "test".to_owned(),
            color_disposition_ref: MOBILE_ANDROID_COLOR_DISPOSITION_REF_V1.to_owned(),
            decoded_byte_count: samples.len(),
            sample_digest_sha256: "cd".repeat(32),
            cache_identity_sha256: "ef".repeat(32),
            samples,
        }
    }

    #[test]
    fn admitted_rgb_and_rgba_materialize_deterministic_argb8888() {
        let rgb = admit_decoded_image_v1(&decoded(
            "rgb",
            8,
            "normal",
            vec![1, 2, 3, 4, 5, 6],
        ))
        .expect("RGB admitted");
        assert_eq!(rgb.argb8888, vec![0xff010203, 0xff040506]);

        let rgba = admit_decoded_image_v1(&decoded(
            "rgba",
            8,
            "normal",
            vec![1, 2, 3, 4, 5, 6, 7, 8],
        ))
        .expect("RGBA admitted");
        assert_eq!(rgba.argb8888, vec![0x04010203, 0x08050607]);
    }

    #[test]
    fn malformed_and_unsupported_codec_fail_closed() {
        let malformed = decode_mobile_image_v1(b"not a jpeg", "image/jpeg")
            .expect_err("malformed JPEG must fail");
        assert_eq!(malformed.code, "jpeg_decode_error");

        let unsupported = decode_mobile_image_v1(b"bytes", "image/gif")
            .expect_err("unsupported codec must fail");
        assert_eq!(unsupported.code, "unsupported_codec");
    }

    #[test]
    fn encoded_and_dimension_limits_are_consumed_from_shared_contract() {
        let encoded = png_fixture(1, 1, &[1, 2, 3, 4]);

        let encoded_limits = DecodeLimitsV1 {
            max_encoded_bytes: encoded.len() - 1,
            ..DecodeLimitsV1::default()
        };
        let encoded_error =
            decode_mobile_image_with_limits(&encoded, "image/png", &encoded_limits)
                .expect_err("encoded byte limit must fail");
        assert_eq!(encoded_error.code, "encoded_bytes_limit");

        let dimension_limits = DecodeLimitsV1 {
            max_dimension_px: 0,
            ..DecodeLimitsV1::default()
        };
        let dimension_error =
            decode_mobile_image_with_limits(&encoded, "image/png", &dimension_limits)
                .expect_err("dimension limit must fail");
        assert_eq!(dimension_error.code, "dimension_limit");
    }

    #[test]
    fn non_normal_orientation_and_non_eight_bit_precision_fail_closed() {
        let orientation = admit_decoded_image_v1(&decoded(
            "rgb",
            8,
            "metadata_only_non_normal",
            vec![0; 6],
        ))
        .expect_err("orientation metadata cannot be silently ignored");
        assert_eq!(orientation.code, "mobile_orientation_unsupported");

        let precision = admit_decoded_image_v1(&decoded(
            "rgb",
            16,
            "normal",
            vec![0; 12],
        ))
        .expect_err("16-bit samples cannot be silently truncated");
        assert_eq!(precision.code, "mobile_precision_unsupported");
    }
}
