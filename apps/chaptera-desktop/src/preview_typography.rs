use eframe::egui;
use pub_viewer::{ViewerTextFragment, ViewerTypographyRun};

const MIN_SOURCE_FONT_SIZE_PX: f32 = 4.0;
const MAX_SOURCE_FONT_SIZE_PX: f32 = 512.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PreviewTypographyUsage {
    pub source_sections: usize,
    pub fallback_sections: usize,
}

#[derive(Debug, Clone, Copy)]
struct SourceSection {
    local_scalar_start: u32,
    local_scalar_end: u32,
    font_size_px: f32,
}

pub(crate) fn layout_fragment(
    typography_runs: &[ViewerTypographyRun],
    fragment: &ViewerTextFragment,
    scene_scale: f32,
    fallback_font_size_px: f32,
    wrap_width_px: f32,
) -> (egui::text::LayoutJob, PreviewTypographyUsage) {
    let Some(fragment_scalar_len) = fragment.scalar_end.checked_sub(fragment.scalar_start) else {
        return fallback_layout(fragment, fallback_font_size_px, wrap_width_px);
    };
    if usize::try_from(fragment_scalar_len).ok() != Some(fragment.text.chars().count()) {
        return fallback_layout(fragment, fallback_font_size_px, wrap_width_px);
    }

    let mut source_sections = typography_runs
        .iter()
        .filter(|run| run.story_id == fragment.story_id)
        .filter_map(|run| {
            let start = run.scalar_start.max(fragment.scalar_start);
            let end = run.scalar_end.min(fragment.scalar_end);
            if start >= end {
                return None;
            }
            Some(SourceSection {
                local_scalar_start: start - fragment.scalar_start,
                local_scalar_end: end - fragment.scalar_start,
                font_size_px: source_text_size_px(run.text_size_emu, scene_scale)?,
            })
        })
        .collect::<Vec<_>>();

    if source_sections.is_empty() {
        return fallback_layout(fragment, fallback_font_size_px, wrap_width_px);
    }

    source_sections.sort_by_key(|section| {
        (
            section.local_scalar_start,
            section.local_scalar_end,
            section.font_size_px.to_bits(),
        )
    });

    let mut previous_end = 0_u32;
    for section in &source_sections {
        if section.local_scalar_start < previous_end {
            return fallback_layout(fragment, fallback_font_size_px, wrap_width_px);
        }
        previous_end = section.local_scalar_end;
    }

    let mut job = egui::text::LayoutJob::default();
    job.wrap.max_width = wrap_width_px.max(1.0);
    let mut cursor = 0_u32;
    let mut source_count = 0_usize;
    let mut fallback_count = 0_usize;

    for section in source_sections {
        if cursor < section.local_scalar_start {
            let Some(text) = scalar_slice(&fragment.text, cursor, section.local_scalar_start) else {
                return fallback_layout(fragment, fallback_font_size_px, wrap_width_px);
            };
            append_section(&mut job, text, fallback_font_size_px);
            fallback_count += 1;
        }

        let Some(text) = scalar_slice(
            &fragment.text,
            section.local_scalar_start,
            section.local_scalar_end,
        ) else {
            return fallback_layout(fragment, fallback_font_size_px, wrap_width_px);
        };
        append_section(&mut job, text, section.font_size_px);
        source_count += 1;
        cursor = section.local_scalar_end;
    }

    if cursor < fragment_scalar_len {
        let Some(text) = scalar_slice(&fragment.text, cursor, fragment_scalar_len) else {
            return fallback_layout(fragment, fallback_font_size_px, wrap_width_px);
        };
        append_section(&mut job, text, fallback_font_size_px);
        fallback_count += 1;
    }

    if job.text != fragment.text {
        return fallback_layout(fragment, fallback_font_size_px, wrap_width_px);
    }

    (
        job,
        PreviewTypographyUsage {
            source_sections: source_count,
            fallback_sections: fallback_count,
        },
    )
}

fn fallback_layout(
    fragment: &ViewerTextFragment,
    fallback_font_size_px: f32,
    wrap_width_px: f32,
) -> (egui::text::LayoutJob, PreviewTypographyUsage) {
    (
        egui::text::LayoutJob::simple(
            fragment.text.clone(),
            egui::FontId::proportional(fallback_font_size_px),
            egui::Color32::BLACK,
            wrap_width_px.max(1.0),
        ),
        PreviewTypographyUsage {
            source_sections: 0,
            fallback_sections: usize::from(!fragment.text.is_empty()),
        },
    )
}

fn append_section(job: &mut egui::text::LayoutJob, text: &str, font_size_px: f32) {
    job.append(
        text,
        0.0,
        egui::TextFormat::simple(
            egui::FontId::proportional(font_size_px),
            egui::Color32::BLACK,
        ),
    );
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

pub(crate) fn source_text_size_px(text_size_emu: u32, scene_scale: f32) -> Option<f32> {
    if text_size_emu == 0 || !scene_scale.is_finite() || scene_scale <= 0.0 {
        return None;
    }
    let px = text_size_emu as f32 * scene_scale;
    px.is_finite()
        .then(|| px.clamp(MIN_SOURCE_FONT_SIZE_PX, MAX_SOURCE_FONT_SIZE_PX))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn fragment() -> ViewerTextFragment {
        serde_json::from_value(json!({
            "story_id": "00000000-0000-0000-0000-000000000001",
            "frame_id": "00000000-0000-0000-0000-000000000002",
            "scalar_start": 10,
            "scalar_end": 16,
            "text": "ABCDEF",
            "line_count": 1
        }))
        .expect("ViewerTextFragment JSON")
    }

    fn run(start: u32, end: u32, points: u32) -> ViewerTypographyRun {
        serde_json::from_value(json!({
            "story_id": "00000000-0000-0000-0000-000000000001",
            "scalar_start": start,
            "scalar_end": end,
            "source_font_name": "Rockwell Condensed",
            "text_size_emu": points * 12_700,
            "render_disposition": "source_identity_known_render_fallback"
        }))
        .expect("ViewerTypographyRun JSON")
    }

    #[test]
    fn mixed_owned_ranges_keep_source_sizes_and_fallback_gaps() {
        let fragment = fragment();
        let runs = [run(10, 12, 24), run(14, 16, 14)];
        let (job, usage) = layout_fragment(
            &runs,
            &fragment,
            1.0 / 12_700.0,
            9.0,
            200.0,
        );

        assert_eq!(job.text, "ABCDEF");
        assert_eq!(job.sections.len(), 3);
        assert_eq!(usage.source_sections, 2);
        assert_eq!(usage.fallback_sections, 1);
        let sizes = job
            .sections
            .iter()
            .map(|section| section.format.font_id.size)
            .collect::<Vec<_>>();
        assert_eq!(sizes, vec![24.0, 9.0, 14.0]);
        assert_eq!(job.sections[0].byte_range, 0..2);
        assert_eq!(job.sections[1].byte_range, 2..4);
        assert_eq!(job.sections[2].byte_range, 4..6);
    }

    #[test]
    fn overlapping_source_ranges_fail_closed_to_fallback() {
        let fragment = fragment();
        let runs = [run(10, 14, 24), run(12, 16, 14)];
        let (job, usage) = layout_fragment(
            &runs,
            &fragment,
            1.0 / 12_700.0,
            9.0,
            200.0,
        );

        assert_eq!(job.sections.len(), 1);
        assert_eq!(job.sections[0].format.font_id.size, 9.0);
        assert_eq!(usage.source_sections, 0);
        assert_eq!(usage.fallback_sections, 1);
    }

    #[test]
    fn scalar_length_mismatch_fails_closed_to_fallback() {
        let mut fragment = fragment();
        fragment.scalar_end = 20;
        let (job, usage) = layout_fragment(
            &[run(10, 20, 24)],
            &fragment,
            1.0 / 12_700.0,
            9.0,
            200.0,
        );

        assert_eq!(job.sections.len(), 1);
        assert_eq!(job.sections[0].format.font_id.size, 9.0);
        assert_eq!(usage.source_sections, 0);
    }

    #[test]
    fn source_size_tracks_scene_scale_without_host_font_lookup() {
        let size = source_text_size_px(24 * 12_700, 2.0 / 12_700.0)
            .expect("positive source size");
        assert!((size - 48.0).abs() < 0.001);
        assert!(source_text_size_px(0, 1.0).is_none());
        assert!(source_text_size_px(24 * 12_700, 0.0).is_none());
    }
}
