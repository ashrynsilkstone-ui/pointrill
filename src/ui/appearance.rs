use eframe::egui::{self, Color32 as Color, FontId, Rect, Stroke, Ui, vec2};
use serde::{Deserialize, Serialize};

use super::bold_family;

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Appearance {
    #[default]
    Graphite,
    Aurora,
    Amethyst,
    Copper,
    Jade,
    Porcelain,
}

#[derive(Clone, Copy)]
pub(super) struct Palette {
    pub base: Color,
    pub rail: Color,
    pub panel: Color,
    pub field: Color,
    pub border: Color,
    pub text: Color,
    pub muted: Color,
    pub accent: Color,
    pub accent_ink: Color,
    pub warning: Color,
    pub graph_secondary: Color,
}

impl Palette {
    pub fn selected(self) -> Color {
        self.panel.blend(self.accent.gamma_multiply(0.14))
    }

    fn hovered(self) -> Color {
        self.field.blend(self.accent.gamma_multiply(0.09))
    }
}

impl Appearance {
    pub const ALL: [Self; 6] = [
        Self::Graphite,
        Self::Aurora,
        Self::Amethyst,
        Self::Copper,
        Self::Jade,
        Self::Porcelain,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Graphite => "Graphit",
            Self::Aurora => "Nordlicht",
            Self::Amethyst => "Amethyst",
            Self::Copper => "Kupfer",
            Self::Jade => "Jade",
            Self::Porcelain => "Porzellan",
        }
    }

    pub fn palette(self) -> Palette {
        let colors = match self {
            Self::Graphite => [
                0x101317, 0x15191E, 0x1C2128, 0x252C35, 0x374450, 0xEFF4FA, 0xACB8C5, 0x83E9B9,
            ],
            Self::Aurora => [
                0x0D1625, 0x111E30, 0x17273C, 0x20354D, 0x385775, 0xEDF6FF, 0xA9C0D8, 0x73DDF8,
            ],
            Self::Amethyst => [
                0x151121, 0x1D172C, 0x261E39, 0x30264A, 0x4C3C6B, 0xF8F1FF, 0xC4B5D8, 0xD0ACFF,
            ],
            Self::Copper => [
                0x1A1513, 0x221B18, 0x2B221D, 0x382B25, 0x584035, 0xFFF4ED, 0xCCB6A9, 0xFFBC85,
            ],
            Self::Jade => [
                0x0D1917, 0x12231F, 0x192D27, 0x233C34, 0x39564A, 0xEBF8F1, 0xA9C8BA, 0xA4DE92,
            ],
            Self::Porcelain => [
                0xF0F3F8, 0xE7EDF5, 0xFFFFFF, 0xE8EEF7, 0xBAC7D9, 0x19263D, 0x475973, 0x2456AF,
            ],
        };
        let [base, rail, panel, field, border, text, muted, accent] = colors.map(rgb);
        let light = self == Self::Porcelain;
        Palette {
            base,
            rail,
            panel,
            field,
            border,
            text,
            muted,
            accent,
            accent_ink: if light { Color::WHITE } else { base },
            warning: rgb(if light { 0xA63632 } else { 0xFFA59A }),
            graph_secondary: rgb(if light { 0x813F9A } else { 0xE8C47E }),
        }
    }

    pub fn load() -> Self {
        crate::profiles::config()
            .and_then(|dir| crate::profiles::read_json(&dir.join("appearance.json")))
            .unwrap_or_default()
    }

    pub fn save(self) -> anyhow::Result<()> {
        crate::profiles::write_json(
            &crate::profiles::config()?.join("appearance.json"),
            &self,
            true,
        )
    }

    pub fn apply(self, context: &egui::Context) {
        context.data_mut(|data| data.insert_temp(appearance_id(), self));
        let colors = self.palette();
        let (mode, visuals) = if self == Self::Porcelain {
            (egui::Theme::Light, egui::Visuals::light())
        } else {
            (egui::Theme::Dark, egui::Visuals::dark())
        };
        let mut style = (*context.style_of(mode)).clone();
        style.visuals = visuals;
        style.visuals.panel_fill = colors.base;
        style.visuals.override_text_color = Some(colors.text);
        style.visuals.window_fill = colors.rail;
        style.visuals.window_stroke = Stroke::new(1.0, colors.border);
        style.visuals.extreme_bg_color = colors.field;
        style.visuals.faint_bg_color = colors.field;
        style.visuals.hyperlink_color = colors.accent;
        style.visuals.warn_fg_color = colors.warning;
        style.visuals.error_fg_color = colors.warning;
        style.visuals.selection.bg_fill = colors.selected();
        style.visuals.selection.stroke = Stroke::new(1.0, colors.accent);
        style.visuals.interact_cursor = Some(egui::CursorIcon::PointingHand);
        style.visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, colors.border);
        style.visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0, colors.muted);
        for state in [
            &mut style.visuals.widgets.inactive,
            &mut style.visuals.widgets.hovered,
            &mut style.visuals.widgets.active,
        ] {
            state.bg_fill = colors.field;
            state.weak_bg_fill = colors.field;
            state.bg_stroke = Stroke::new(1.0, colors.border);
            state.fg_stroke = Stroke::new(1.0, colors.text);
            state.corner_radius = 7.into();
        }
        style.visuals.widgets.hovered.bg_fill = colors.hovered();
        style.visuals.widgets.hovered.weak_bg_fill = colors.hovered();
        style.visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, colors.accent);
        style.visuals.widgets.active.bg_fill = colors.selected();
        style.visuals.widgets.active.weak_bg_fill = colors.selected();
        style.visuals.widgets.active.bg_stroke = Stroke::new(1.0, colors.accent);
        style.spacing.item_spacing = vec2(7.0, 6.0);
        style.spacing.button_padding = vec2(9.0, 6.0);
        style.spacing.interact_size.y = 31.0;
        style
            .text_styles
            .insert(egui::TextStyle::Body, FontId::proportional(14.0));
        context.set_style_of(mode, style);
        context.set_theme(mode);
        context.request_repaint();
    }

    pub fn preview(self, ui: &mut Ui) {
        let colors = self.palette();
        let width = ui.available_width().min(342.0);
        let (window, _) = ui.allocate_exact_size(vec2(width, 156.0), egui::Sense::hover());
        let paint = ui.painter_at(window);
        let at = |x, y| window.min + vec2(x, y);
        let text = |x, y, label, size, color| {
            paint.text(
                at(x, y),
                egui::Align2::LEFT_CENTER,
                label,
                FontId::new(size, bold_family()),
                color,
            );
        };

        paint.rect_filled(window, 11, colors.base);
        paint.rect_filled(
            Rect::from_min_size(at(1.0, 1.0), vec2(width - 2.0, 32.0)),
            9,
            colors.rail,
        );
        paint.rect_stroke(
            window,
            11,
            Stroke::new(1.0, colors.border),
            egui::StrokeKind::Inside,
        );
        paint.circle_filled(at(12.0, 16.0), 3.0, colors.accent);
        text(23.0, 16.0, "Pointrill", 12.0, colors.text);
        paint.line_segment(
            [at(1.0, 32.0), at(width - 1.0, 32.0)],
            Stroke::new(1.0, colors.border),
        );

        paint.rect_filled(
            Rect::from_min_size(at(9.0, 48.0), vec2(79.0, 25.0)),
            5,
            colors.selected(),
        );
        text(19.0, 60.0, "Sensor", 11.0, colors.accent);
        text(19.0, 86.0, "Beleuchtung", 10.0, colors.muted);

        let card = Rect::from_min_size(at(99.0, 44.0), vec2((width - 112.0).max(0.0), 98.0));
        paint.rect(
            card,
            8,
            colors.panel,
            Stroke::new(1.0, colors.border),
            egui::StrokeKind::Inside,
        );
        text(111.0, 61.0, "Empfindlichkeit", 11.0, colors.muted);
        text(111.0, 84.0, "2400 DPI", 19.0, colors.text);
        paint.rect_filled(
            Rect::from_min_size(at(111.0, 107.0), vec2(68.0, 23.0)),
            5,
            colors.accent,
        );
        text(120.0, 118.0, "Stufe 3", 11.0, colors.accent_ink);
    }
}

fn rgb(value: u32) -> Color {
    Color::from_rgb((value >> 16) as u8, (value >> 8) as u8, value as u8)
}

fn appearance_id() -> egui::Id {
    egui::Id::unique("pointrill-appearance")
}

pub(super) fn current(context: &egui::Context) -> Appearance {
    context.data(|data| data.get_temp(appearance_id()).unwrap_or_default())
}

pub(super) fn colors(ui: &Ui) -> Palette {
    current(ui.ctx()).palette()
}
