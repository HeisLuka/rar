//! Desktop egui execution backend for source-neutral Chaptera render plans.
//!
//! This module owns only how already-resolved document paint facts are executed
//! by egui. Product interaction state (selection, caret, drag/resize admission,
//! EditorSession state, commands and product chrome) stays in the shell.

use chaptera_viewer_render_plan::{NodeRenderPlanV1, RenderTextFragmentV1};
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
    let layout_job = layout_document_text(
        fragment,
        scene_scale,
        text_clip_rect.width().max(1.0_f32),
    );
    let galley = text_painter.layout_job(layout_job);
    let text_clipped = preview_text_height_is_clipped(galley.size().y, text_clip_rect.height());
    text_painter.galley(text_clip_rect.min, galley, egui::Color32::BLACK);

    NodePaintOutcome { text_clipped }
}

fn layout_document_text(
    fragment: &RenderTextFragmentV1,
    scene_scale: f32,
    wrap_width_px: f32,
) -> egui::text::LayoutJob {
    let fallback = crate::fallback_font::font_id_for_scene_scale(scene_scale);
    let fallback_job = || {
        egui::text::LayoutJob::simple(
            fragment.text.clone(),
            fallback.clone(),
            egui::Color32::BLACK,
            wrap_width_px.max(1.0),
        )
    };

    let Some(fragment_scalar_len) = fragment.scalar_end.checked_sub(fragment.scalar_start) else {
        return fallback_job();
    };
    if usize::try_from(fragment_scalar_len).ok() != Some(fragment.text.chars().count()) {
        return fallback_job();
    }

    let mut source_sections = fragment
        .typography
        .iter()
        .filter_map(|run| {
            let start = run.scalar_start.max(fragment.scalar_start);
            let end = run.scalar_end.min(fragment.scalar_end);
            if start >= end {
                return None;
            }
            let size = source_text_size_px(run.text_size_emu, scene_scale)?;
            Some((start - fragment.scalar_start, end - fragment.scalar_start, size))
        })
        .collect::<Vec<_>>();

    if source_sections.is_empty() {
        return fallback_job();
    }
    source_sections.sort_by_key(|(start, end, size)| (*start, *end, size.to_bits()));
    if source_sections
        .windows(2)
        .any(|pair| pair[1].0 < pair[0].1)
    {
        return fallback_job();
    }

    let mut job = egui::text::LayoutJob::default();
    job.wrap.max_width = wrap_width_px.max(1.0);
    let mut cursor = 0_u32;

    for (start, end, size) in source_sections {
        if cursor < start {
            let Some(text) = scalar_slice(&fragment.text, cursor, start) else {
                return fallback_job();
            };
            append_text_section(&mut job, text, fallback.clone());
        }
        let Some(text) = scalar_slice(&fragment.text, start, end) else {
            return fallback_job();
        };
        append_text_section(
            &mut job,
            text,
            egui::FontId::new(size, crate::fallback_font::family()),
        );
        cursor = end;
    }

    if cursor < fragment_scalar_len {
        let Some(text) = scalar_slice(&fragment.text, cursor, fragment_scalar_len) else {
            return fallback_job();
        };
        append_text_section(&mut job, text, fallback);
    }

    if job.text != fragment.text {
        return fallback_job();
    }
    job
}

fn append_text_section(job: &mut egui::text::LayoutJob, text: &str, font_id: egui::FontId) {
    job.append(
        text,
        0.0,
        egui::TextFormat::simple(font_id, egui::Color32::BLACK),
    );
}

fn source_text_size_px(text_size_emu: u32, scene_scale: f32) -> Option<f32> {
    const MIN_PX: f32 = 4.0;
    const MAX_PX: f32 = 512.0;
    if text_size_emu == 0 || !scene_scale.is_finite() || scene_scale <= 0.0 {
        return None;
    }
    let px = text_size_emu as f32 * scene_scale;
    px.is_finite().then(|| px.clamp(MIN_PX, MAX_PX))
}

fn scalar_slice(text: &str, start: u32, end: u32) -> Option<&str> {
    if start > end {
        return None;
    }
    let start = scalar_boundary_to_byte(text, start)?;
    let end = scalar_boundary_to_byte(text, end)?;
    text.get(start..end)
}

fn scalar_boundary_to_byte(text: &str, target: u32) -> Option<usize> {
    if target == 0 {
        return Some(0);
    }
    let mut scalar = 0_u32;
    for (byte_offset, _) in text.char_indices() {
        if scalar == target {
            return Some(byte_offset);
        }
        scalar = scalar.checked_add(1)?;
    }
    (scalar == target).then_some(text.len())
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

    #[test]
    fn typography_sections_change_size_but_keep_pinned_fallback_family() {
        let fragment: RenderTextFragmentV1 = serde_json::from_value(serde_json::json!({
            "story_id": "00000000-0000-0000-0000-000000000001",
            "scalar_start": 0,
            "scalar_end": 6,
            "text": "ABCDEF",
            "line_count": 1,
            "typography": [
                {
                    "scalar_start": 0,
                    "scalar_end": 2,
                    "source_font_name": "Rockwell Condensed",
                    "text_size_emu": 304800,
                    "font_inherited": false,
                    "size_inherited": false
                },
                {
                    "scalar_start": 4,
                    "scalar_end": 6,
                    "source_font_name": "Arial",
                    "text_size_emu": 177800,
                    "font_inherited": true,
                    "size_inherited": true
                }
            ]
        }))
        .expect("render text fragment");

        let scene_scale = 1.0 / 12_700.0;
        let job = layout_document_text(&fragment, scene_scale, 400.0);
        assert_eq!(job.text, "ABCDEF");
        assert_eq!(job.sections.len(), 3);
        let sizes = job
            .sections
            .iter()
            .map(|section| section.format.font_id.size)
            .collect::<Vec<_>>();
        assert_eq!(sizes, vec![24.0, crate::fallback_font::screen_font_size(scene_scale), 14.0]);
        assert!(job
            .sections
            .iter()
            .all(|section| section.format.font_id.family == crate::fallback_font::family()));
    }

    #[test]
    fn overlapping_typography_fails_closed_to_one_fallback_section() {
        let fragment: RenderTextFragmentV1 = serde_json::from_value(serde_json::json!({
            "story_id": "00000000-0000-0000-0000-000000000001",
            "scalar_start": 0,
            "scalar_end": 4,
            "text": "ABCD",
            "line_count": 1,
            "typography": [
                {"scalar_start": 0, "scalar_end": 3, "source_font_name": "A", "text_size_emu": 152400, "font_inherited": false, "size_inherited": false},
                {"scalar_start": 2, "scalar_end": 4, "source_font_name": "B", "text_size_emu": 304800, "font_inherited": false, "size_inherited": false}
            ]
        }))
        .expect("render text fragment");
        let job = layout_document_text(&fragment, 1.0 / 12_700.0, 400.0);
        assert_eq!(job.sections.len(), 1);
        assert_eq!(job.sections[0].format.font_id, crate::fallback_font::font_id_for_scene_scale(1.0 / 12_700.0));
    }
}
