use sha2::{Digest, Sha256};

pub const RESOURCE_ID: &str = "chaptera.desktop.fallback-font.ubuntu-light.v1";
pub const FAMILY_NAME: &str = "Ubuntu Light";
pub const DISPOSITION: &str = "fallback_not_source_font";
pub const UPSTREAM_CRATE: &str = "epaint_default_fonts";
pub const UPSTREAM_VERSION: &str = "0.31.1";
pub const UPSTREAM_TAG_COMMIT: &str = "1669e52a7ccfc3489c1b0999b9ed48894a0b3887";
pub const LICENSE_ID: &str = "Ubuntu-font-1.0";
pub const COPYRIGHT_NOTICE: &str =
    "Copyright 2011 Canonical Ltd. Licensed under the Ubuntu Font Licence 1.0";
pub const EXPECTED_SHA256: &str =
    "80307b8da7649aa4ee4d484b232140e3ce1ec0ca093073d3c53c8f5a5ced7a70";
pub const EXPECTED_BYTE_LEN: usize = 361_676;

/// Bounded Chaptera V0 fallback metrics.
///
/// 9 typographic points map to 12 screen points at the existing 96-dpi 100%
/// canvas scale. The 11.25pt line height preserves the explicit Viewer
/// fallback line-height law while authoritative shaping supplies advances.
pub const FONT_SIZE_EMU: i64 = 114_300;
pub const LINE_HEIGHT_EMU: i64 = 142_875;

pub fn bytes() -> &'static [u8] {
    epaint_default_fonts::UBUNTU_LIGHT
}

pub fn sha256_hex() -> String {
    let digest = Sha256::digest(bytes());
    format!("{digest:x}")
}

pub fn validate() -> Result<(), String> {
    if bytes().len() != EXPECTED_BYTE_LEN {
        return Err(format!(
            "fallback font byte length mismatch for {RESOURCE_ID}: expected {EXPECTED_BYTE_LEN}, got {}",
            bytes().len()
        ));
    }
    let actual = sha256_hex();
    if actual != EXPECTED_SHA256 {
        return Err(format!(
            "fallback font fingerprint mismatch for {RESOURCE_ID}: expected {EXPECTED_SHA256}, got {actual}"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_resource_identity_is_pinned() {
        assert_eq!(FAMILY_NAME, "Ubuntu Light");
        assert_eq!(DISPOSITION, "fallback_not_source_font");
        assert_eq!(UPSTREAM_CRATE, "epaint_default_fonts");
        assert_eq!(UPSTREAM_VERSION, "0.31.1");
        assert_eq!(
            UPSTREAM_TAG_COMMIT,
            "1669e52a7ccfc3489c1b0999b9ed48894a0b3887"
        );
        assert_eq!(LICENSE_ID, "Ubuntu-font-1.0");
        assert_eq!(
            COPYRIGHT_NOTICE,
            "Copyright 2011 Canonical Ltd. Licensed under the Ubuntu Font Licence 1.0"
        );
        assert_eq!(bytes().len(), EXPECTED_BYTE_LEN);
    }

    #[test]
    fn exact_bytes_match_pinned_fingerprint() {
        validate().expect("pinned Ubuntu Light resource must validate");
        assert_eq!(sha256_hex(), EXPECTED_SHA256);
    }
}
