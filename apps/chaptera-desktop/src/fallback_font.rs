use eframe::egui;
use sha2::{Digest, Sha256};
use std::sync::Arc;

pub const RESOURCE_ID: &str = "chaptera.desktop.fallback-font.ubuntu-light.v1";
pub const FAMILY_NAME: &str = "Ubuntu Light";
pub const DISPOSITION: &str = "fallback_not_source_font";
pub const UPSTREAM_CRATE: &str = "epaint_default_fonts";
pub const UPSTREAM_VERSION: &str = "0.31.1";
pub const LICENSE_ID: &str = "Ubuntu-font-1.0";

// Filled from the exact epaint_default_fonts 0.31.1 bytes by the dedicated gate.
pub const EXPECTED_SHA256: &str = "PENDING_SHA256";

pub fn bytes() -> &'static [u8] {
    epaint_default_fonts::UBUNTU_LIGHT
}

pub fn sha256_hex() -> String {
    let digest = Sha256::digest(bytes());
    format!("{digest:x}")
}

pub fn validate() -> Result<(), String> {
    let actual = sha256_hex();
    if actual != EXPECTED_SHA256 {
        return Err(format!(
            "fallback font fingerprint mismatch for {RESOURCE_ID}: expected {EXPECTED_SHA256}, got {actual}"
        ));
    }
    Ok(())
}

pub fn install(ctx: &egui::Context) -> Result<(), String> {
    validate()?;

    let mut fonts = egui::FontDefinitions::default();
    let key = RESOURCE_ID.to_owned();
    fonts.font_data.insert(
        key.clone(),
        Arc::new(egui::FontData::from_static(bytes())),
    );
    fonts
        .families
        .entry(egui::FontFamily::Proportional)
        .or_default()
        .retain(|name| name != &key);
    fonts
        .families
        .entry(egui::FontFamily::Proportional)
        .or_default()
        .insert(0, key);
    ctx.set_fonts(fonts);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resource_identity_is_explicit_and_not_source_font() {
        assert_eq!(FAMILY_NAME, "Ubuntu Light");
        assert_eq!(DISPOSITION, "fallback_not_source_font");
        assert_eq!(UPSTREAM_CRATE, "epaint_default_fonts");
        assert_eq!(UPSTREAM_VERSION, "0.31.1");
        assert_eq!(LICENSE_ID, "Ubuntu-font-1.0");
        assert!(!bytes().is_empty());
    }

    #[test]
    fn exact_bytes_match_pinned_fingerprint() {
        assert_eq!(sha256_hex(), EXPECTED_SHA256);
    }
}
