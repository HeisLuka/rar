use eframe::egui;

pub const WINDOW_BG: egui::Color32 = egui::Color32::from_rgb(242, 245, 248);
pub const PANEL_BG: egui::Color32 = egui::Color32::from_rgb(249, 250, 252);
pub const CANVAS_BG: egui::Color32 = egui::Color32::from_rgb(229, 233, 237);
pub const BORDER: egui::Color32 = egui::Color32::from_rgb(213, 218, 224);
pub const TEXT: egui::Color32 = egui::Color32::from_rgb(28, 34, 40);
pub const MUTED_TEXT: egui::Color32 = egui::Color32::from_rgb(101, 111, 122);
pub const ACCENT: egui::Color32 = egui::Color32::from_rgb(20, 118, 177);
pub const ACCENT_SOFT: egui::Color32 = egui::Color32::from_rgb(224, 239, 249);
pub const SUCCESS: egui::Color32 = egui::Color32::from_rgb(20, 128, 82);
pub const WARNING: egui::Color32 = egui::Color32::from_rgb(181, 113, 21);
pub const DANGER: egui::Color32 = egui::Color32::from_rgb(185, 56, 56);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum InspectorTab {
    #[default]
    Document,
    Text,
    Diagnostics,
}

pub fn configure_context(ctx: &egui::Context) {
    let mut visuals = egui::Visuals::light();
    visuals.panel_fill = PANEL_BG;
    visuals.window_fill = PANEL_BG;
    visuals.extreme_bg_color = egui::Color32::WHITE;
    visuals.faint_bg_color = WINDOW_BG;
    visuals.selection.bg_fill = ACCENT;
    visuals.selection.stroke.color = egui::Color32::WHITE;
    visuals.widgets.inactive.bg_fill = egui::Color32::TRANSPARENT;
    visuals.widgets.inactive.weak_bg_fill = egui::Color32::TRANSPARENT;
    visuals.widgets.inactive.fg_stroke.color = TEXT;
    visuals.widgets.hovered.bg_fill = egui::Color32::from_rgb(236, 242, 247);
    visuals.widgets.hovered.weak_bg_fill = egui::Color32::from_rgb(236, 242, 247);
    visuals.widgets.hovered.fg_stroke.color = TEXT;
    visuals.widgets.active.bg_fill = ACCENT_SOFT;
    visuals.widgets.active.weak_bg_fill = ACCENT_SOFT;
    visuals.widgets.active.fg_stroke.color = TEXT;
    visuals.window_stroke = egui::Stroke::new(1.0, BORDER);
    ctx.set_visuals(visuals);

    let mut style = (*ctx.style()).clone();
    style.spacing.item_spacing = egui::vec2(8.0, 8.0);
    style.spacing.button_padding = egui::vec2(10.0, 7.0);
    style.spacing.indent = 16.0;
    ctx.set_style(style);
}

pub fn toolbar_button(
    ui: &mut egui::Ui,
    glyph: &str,
    label: &str,
    selected: bool,
) -> egui::Response {
    let fill = if selected {
        ACCENT_SOFT
    } else {
        egui::Color32::TRANSPARENT
    };
    let stroke = if selected {
        egui::Stroke::new(1.0, egui::Color32::from_rgb(186, 218, 238))
    } else {
        egui::Stroke::new(0.0, egui::Color32::TRANSPARENT)
    };
    ui.add(
        egui::Button::new(
            egui::RichText::new(format!("{glyph}\n{label}"))
                .size(12.0)
                .color(TEXT),
        )
        .min_size(egui::vec2(74.0, 56.0))
        .fill(fill)
        .stroke(stroke),
    )
}

pub fn compact_toolbar_button(
    ui: &mut egui::Ui,
    glyph: &str,
    label: &str,
    selected: bool,
) -> egui::Response {
    let fill = if selected {
        ACCENT_SOFT
    } else {
        egui::Color32::TRANSPARENT
    };
    ui.add(
        egui::Button::new(
            egui::RichText::new(format!("{glyph}  {label}"))
                .size(12.0)
                .color(TEXT),
        )
        .min_size(egui::vec2(86.0, 30.0))
        .fill(fill),
    )
}

pub fn inspector_tab(ui: &mut egui::Ui, label: &str, selected: bool) -> egui::Response {
    let text = if selected {
        egui::RichText::new(label).strong().color(ACCENT)
    } else {
        egui::RichText::new(label).color(MUTED_TEXT)
    };
    ui.add(
        egui::Button::new(text)
            .fill(egui::Color32::TRANSPARENT)
            .stroke(egui::Stroke::new(0.0, egui::Color32::TRANSPARENT)),
    )
}

pub fn section_label(ui: &mut egui::Ui, label: impl Into<String>) {
    ui.label(
        egui::RichText::new(label.into())
            .size(12.0)
            .strong()
            .color(TEXT),
    );
}

pub fn muted(ui: &mut egui::Ui, text: impl Into<String>) {
    ui.label(
        egui::RichText::new(text.into())
            .size(11.0)
            .color(MUTED_TEXT),
    );
}

pub fn status_color(status: &str) -> egui::Color32 {
    match status {
        "Supported" => SUCCESS,
        "Partial" => WARNING,
        "Unsupported" => DANGER,
        _ => MUTED_TEXT,
    }
}

pub fn panel_separator(ui: &mut egui::Ui) {
    ui.add_space(2.0);
    ui.separator();
    ui.add_space(2.0);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inspector_tab_defaults_to_document() {
        assert_eq!(InspectorTab::default(), InspectorTab::Document);
    }

    #[test]
    fn fidelity_status_colors_are_stable() {
        assert_eq!(status_color("Supported"), SUCCESS);
        assert_eq!(status_color("Partial"), WARNING);
        assert_eq!(status_color("Unsupported"), DANGER);
        assert_eq!(status_color("unknown"), MUTED_TEXT);
    }
}
