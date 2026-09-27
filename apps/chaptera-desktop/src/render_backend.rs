//! Desktop egui execution backend for source-neutral Chaptera render plans.
//!
//! This module owns only how already-resolved document paint facts are executed
//! by egui. Product interaction state (selection, caret, drag/resize admission,
//! EditorSession state, commands and product chrome) stays in the shell.

use chaptera_viewer_render_plan::NodeRenderPlanV1;
use eframe::egui;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct NodePaintOutcome {
    pub text_clipped: bool,
}

pub fn paint_page_surface(painter: &egui::Painter, page_rect: egui::Rect) {
    painter.rect_filled(page_rect, 0, egui::Color32::WHITE);
    painter.rect_stroke(
        page_rect,
        0,
        egui::Stroke::new(1.0_f32, egui::Color32::DARK_GRAY),
        egui::StrokeKind::Inside,
    );
}

pub fn physical_rect_to_egui(
    page_rect: egui::Rect,
    scene_scale: f32,
    x_emu: i64,
    y_emu: i64,
    width_emu: i64,
    height_emu: i64,
) -> Option<egui::Rect> {
    if !scene_scale.is_finite() || scene_scale <= 0.0 || width_emu <= 0 || height_emu <= 0 {
        return None;
    }

    let min = egui::pos2(
        page_rect.left() + x_emu as f32 * scene_scale,
        page_rect.top() + y_emu as f32 * scene_scale,
    );
    let size = egui::vec2(
        width_emu as f32 * scene_scale,
        height_emu as f32 * scene_scale,
    );
    Some(egui::Rect::from_min_size(min, size))
}

/// Paints document-owned layers that occur before shell/debug overlays.
///
/// The shell resolves temporary authoring preview state (for example a
/// replacement texture) before calling this function. No Editor state crosses
/// this boundary.
pub fn paint_document_node_base(
    painter: &egui::Painter,
    node: &NodeRenderPlanV1,
    node_rect: egui::Rect,
    texture: Option<egui::TextureId>,
) {
    if let Some(rgb) = node.solid_fill_rgb {
        painter.rect_filled(
            node_rect,
            0,
            egui::Color32::from_rgb(rgb[0], rgb[1], rgb[2]),
        );
    }

    if let Some(texture) = texture {
        painter.image(
            texture,
            node_rect.shrink(1.0),
            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
            egui::Color32::WHITE,
        );
    }
}

/// Paints document-owned layers that occur after shell/debug overlays.
///
/// Text overflow is returned as a fact. The product shell still owns the
/// user-facing red warning/affordance because it is preview UI rather than
/// document paint.
pub fn paint_document_node_foreground(
    painter: &egui::Painter,
    node: &NodeRenderPlanV1,
    node_rect: egui::Rect,
    scene_scale: f32,
) -> NodePaintOutcome {
    if let Some(line) = node.solid_line.as_ref() {
        let line_width_px = line.width_emu as f32 * scene_scale;
        if line_width_px > 0.0_f32 {
            painter.rect_stroke(
                node_rect,
                0,
                egui::Stroke::new(
                    line_width_px,
                    egui::Color32::from_rgb(line.rgb[0], line.rgb[1], line.rgb[2]),
                ),
                egui::StrokeKind::Inside,
            );
        }
    }

    let Some(fragment) = node
        .text
        .as_ref()
        .filter(|fragment| !fragment.text.is_empty())
    else {
        return NodePaintOutcome::default();
    };

    let text_clip_rect = node_rect.shrink(2.0);
    let text_painter = painter.with_clip_rect(text_clip_rect);
    let base_font = crate::fallback_font::font_id_for_scene_scale(scene_scale);

    // The current Viewer text-flow boundary exposes a recovered line budget,
    // but not Publisher-exact font runs yet. Use the frame geometry and that
    // line budget to keep fallback text readable without painting outside the
    // authored frame. This is presentation-only: it does not alter canonical
    // Story text, frame ownership or text-flow authority.
    let recovered_lines = fragment.line_count.max(1) as f32;
    let line_height_ratio = 1.25_f32;
    let frame_driven_size = (text_clip_rect.height() / recovered_lines / line_height_ratio)
        .clamp(base_font.size * 0.75_f32, base_font.size * 4.0_f32);
    let mut fitted_size = frame_driven_size;

    // A single recovered line should stay on one visual line when possible.
    // This matters for titles/bylines where height-only fitting can otherwise
    // produce a large font that immediately wraps.
    if fragment.line_count <= 1 {
        for _ in 0..8 {
            let one_line = text_painter.layout_no_wrap(
                fragment.text.clone(),
                egui::FontId::new(fitted_size, base_font.family.clone()),
                egui::Color32::BLACK,
            );
            if one_line.size().x <= text_clip_rect.width().max(1.0_f32) {
                break;
            }
            let next_size = (fitted_size * 0.88_f32).max(base_font.size * 0.65_f32);
            if (next_size - fitted_size).abs() < f32::EPSILON {
                break;
            }
            fitted_size = next_size;
        }
    }

    let mut galley = text_painter.layout(
        fragment.text.clone(),
        egui::FontId::new(fitted_size, base_font.family.clone()),
        egui::Color32::BLACK,
        text_clip_rect.width().max(1.0_f32),
    );
    for _ in 0..8 {
        if !preview_text_height_is_clipped(galley.size().y, text_clip_rect.height()) {
            break;
        }
        let next_size = (fitted_size * 0.88_f32).max(base_font.size * 0.65_f32);
        if (next_size - fitted_size).abs() < f32::EPSILON {
            break;
        }
        fitted_size = next_size;
        galley = text_painter.layout(
            fragment.text.clone(),
            egui::FontId::new(fitted_size, base_font.family.clone()),
            egui::Color32::BLACK,
            text_clip_rect.width().max(1.0_f32),
        );
    }

    let text_clipped = preview_text_height_is_clipped(galley.size().y, text_clip_rect.height());
    text_painter.galley(text_clip_rect.min, galley, egui::Color32::BLACK);

    NodePaintOutcome { text_clipped }
}

fn preview_text_height_is_clipped(galley_height: f32, clip_height: f32) -> bool {
    const EPSILON_PX: f32 = 0.5;
    galley_height > clip_height + EPSILON_PX
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn physical_rect_conversion_rejects_invalid_paint_geometry() {
        let page = egui::Rect::from_min_size(egui::pos2(100.0, 50.0), egui::vec2(400.0, 300.0));

        assert!(physical_rect_to_egui(page, 0.0, 1, 1, 10, 10).is_none());
        assert!(physical_rect_to_egui(page, 1.0, 1, 1, 0, 10).is_none());
        assert!(physical_rect_to_egui(page, 1.0, 1, 1, 10, -1).is_none());
    }

    #[test]
    fn physical_rect_conversion_uses_page_origin_and_scene_scale() {
        let page = egui::Rect::from_min_size(egui::pos2(100.0, 50.0), egui::vec2(400.0, 300.0));
        let rect = physical_rect_to_egui(page, 0.5, 20, 30, 100, 80).expect("valid physical rect");

        assert_eq!(rect.min, egui::pos2(110.0, 65.0));
        assert_eq!(rect.size(), egui::vec2(50.0, 40.0));
    }

    #[test]
    fn preview_text_clipping_uses_visible_galley_height_only() {
        assert!(!preview_text_height_is_clipped(100.0, 100.0));
        assert!(!preview_text_height_is_clipped(100.4, 100.0));
        assert!(preview_text_height_is_clipped(100.6, 100.0));
    }
}
