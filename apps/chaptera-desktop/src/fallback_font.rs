use chaptera_desktop_fallback_font_resource as resource;
use eframe::egui;
use std::sync::Arc;

pub fn family() -> egui::FontFamily {
    egui::FontFamily::Name(resource::RESOURCE_ID.into())
}

pub fn screen_font_size(scene_scale: f32) -> f32 {
    resource::FONT_SIZE_EMU as f32 * scene_scale
}

pub fn screen_line_height(scene_scale: f32) -> f32 {
    resource::LINE_HEIGHT_EMU as f32 * scene_scale
}

pub fn font_id_for_scene_scale(scene_scale: f32) -> egui::FontId {
    egui::FontId::new(screen_font_size(scene_scale), family())
}

pub fn install(ctx: &egui::Context) -> Result<(), String> {
    resource::validate()?;

    let mut fonts = egui::FontDefinitions::default();
    let key = resource::RESOURCE_ID.to_owned();
    fonts.font_data.insert(
        key.clone(),
        Arc::new(egui::FontData::from_static(resource::bytes())),
    );
    fonts.families.insert(family(), vec![key]);
    ctx.set_fonts(fonts);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn document_metrics_match_existing_canvas_scale_at_100_percent() {
        let scene_scale = 96.0_f32 / 914_400.0_f32;
        assert!((screen_font_size(scene_scale) - 12.0).abs() < f32::EPSILON);
        assert!((screen_line_height(scene_scale) - 15.0).abs() < f32::EPSILON);
    }

    #[test]
    fn chaptera_canvas_family_is_isolated_from_general_proportional_ui() {
        assert_ne!(family(), egui::FontFamily::Proportional);
        let context = egui::Context::default();
        install(&context).expect("pinned font should install");
    }

    #[test]
    fn paint_family_uses_the_shared_product_resource_identity() {
        assert_eq!(
            family(),
            egui::FontFamily::Name(resource::RESOURCE_ID.into())
        );
        assert_eq!(resource::DISPOSITION, "fallback_not_source_font");
        assert_eq!(resource::sha256_hex(), resource::EXPECTED_SHA256);
    }
}
