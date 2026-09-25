//! 색·글꼴·간격. 라이트/다크 모두 시스템 설정을 따른다.

use std::sync::Arc;

use eframe::egui::{
    self, Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Margin, Shadow, Stroke,
    TextStyle, Theme, Visuals, vec2,
};

/// 굵은 글꼴 가족 이름.
pub const BOLD: &str = "bold";

pub fn bold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(BOLD.into()))
}

pub fn regular(size: f32) -> FontId {
    FontId::new(size, FontFamily::Proportional)
}

#[derive(Clone, Copy)]
pub struct Palette {
    pub bg: Color32,
    pub card: Color32,
    pub border: Color32,
    pub input: Color32,
    pub subtle: Color32,
    pub subtle_hover: Color32,
    pub text: Color32,
    pub muted: Color32,
    pub accent: Color32,
    pub accent_hover: Color32,
    pub accent_press: Color32,
    pub success: Color32,
    pub warning: Color32,
    pub danger: Color32,
}

const fn rgb(r: u8, g: u8, b: u8) -> Color32 {
    Color32::from_rgb(r, g, b)
}

impl Palette {
    pub const LIGHT: Palette = Palette {
        bg: rgb(244, 245, 248),
        card: rgb(255, 255, 255),
        border: rgb(228, 231, 237),
        input: rgb(246, 247, 250),
        subtle: rgb(239, 241, 245),
        subtle_hover: rgb(229, 232, 238),
        text: rgb(21, 24, 33),
        muted: rgb(107, 114, 128),
        accent: rgb(79, 70, 229),
        accent_hover: rgb(67, 56, 202),
        accent_press: rgb(55, 48, 163),
        success: rgb(21, 128, 61),
        warning: rgb(180, 83, 9),
        danger: rgb(220, 38, 38),
    };

    pub const DARK: Palette = Palette {
        bg: rgb(15, 17, 21),
        card: rgb(24, 27, 34),
        border: rgb(39, 43, 53),
        input: rgb(17, 19, 24),
        subtle: rgb(34, 38, 48),
        subtle_hover: rgb(44, 49, 61),
        text: rgb(231, 233, 238),
        muted: rgb(140, 147, 163),
        accent: rgb(99, 102, 241),
        accent_hover: rgb(129, 140, 248),
        accent_press: rgb(79, 70, 229),
        success: rgb(34, 197, 94),
        warning: rgb(245, 158, 11),
        danger: rgb(248, 113, 113),
    };

    pub fn of(ui: &egui::Ui) -> Palette {
        if ui.visuals().dark_mode { Palette::DARK } else { Palette::LIGHT }
    }
}

/// 색을 반투명하게 (상태 배지·알림 배경용).
pub fn tint(c: Color32, alpha: u8) -> Color32 {
    Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), alpha)
}

pub fn install(ctx: &egui::Context) {
    install_fonts(ctx);
    for (theme, p) in [(Theme::Light, Palette::LIGHT), (Theme::Dark, Palette::DARK)] {
        ctx.set_visuals_of(theme, visuals(theme, &p));
        ctx.style_mut_of(theme, |s| {
            s.spacing.item_spacing = vec2(8.0, 8.0);
            s.spacing.button_padding = vec2(14.0, 7.0);
            s.spacing.interact_size.y = 34.0;
            s.spacing.window_margin = Margin::same(24);
            s.spacing.menu_margin = Margin::same(6);
            s.text_styles = [
                (TextStyle::Heading, bold(22.0)),
                (TextStyle::Body, regular(14.5)),
                (TextStyle::Button, regular(14.0)),
                (TextStyle::Small, regular(12.0)),
                (TextStyle::Monospace, FontId::monospace(13.0)),
            ]
            .into();
        });
    }
}

fn visuals(theme: Theme, p: &Palette) -> Visuals {
    let mut v = if theme == Theme::Dark { Visuals::dark() } else { Visuals::light() };
    v.panel_fill = p.bg;
    v.window_fill = p.card;
    v.window_stroke = Stroke::new(1.0, p.border);
    v.window_corner_radius = CornerRadius::same(16);
    v.menu_corner_radius = CornerRadius::same(10);
    let shadow_alpha = if theme == Theme::Dark { 110 } else { 36 };
    v.window_shadow = Shadow { offset: [0, 10], blur: 32, spread: 0, color: Color32::from_black_alpha(shadow_alpha) };
    v.popup_shadow = Shadow { offset: [0, 6], blur: 18, spread: 0, color: Color32::from_black_alpha(shadow_alpha) };
    v.extreme_bg_color = p.input;
    v.text_edit_bg_color = Some(p.input);
    v.faint_bg_color = p.subtle;
    v.hyperlink_color = p.accent;
    v.selection.bg_fill = tint(p.accent, 70);
    v.selection.stroke = Stroke::new(1.5, p.accent);

    let w = &mut v.widgets;
    for state in [&mut w.noninteractive, &mut w.inactive, &mut w.hovered, &mut w.active, &mut w.open] {
        state.corner_radius = CornerRadius::same(9);
        state.fg_stroke = Stroke::new(1.0, p.text);
    }
    w.noninteractive.bg_stroke = Stroke::new(1.0, p.border);
    w.noninteractive.bg_fill = p.card;
    w.inactive.bg_fill = p.subtle;
    w.inactive.weak_bg_fill = p.subtle;
    w.inactive.bg_stroke = Stroke::new(1.0, p.border);
    w.hovered.bg_fill = p.subtle_hover;
    w.hovered.weak_bg_fill = p.subtle_hover;
    w.hovered.bg_stroke = Stroke::new(1.0, p.border);
    w.hovered.expansion = 0.0;
    w.active.bg_fill = p.subtle_hover;
    w.active.weak_bg_fill = p.subtle_hover;
    w.active.bg_stroke = Stroke::new(1.5, p.accent);
    w.active.expansion = 0.0;
    w.open.weak_bg_fill = p.subtle_hover;
    v
}

fn install_fonts(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    fonts.font_data.insert(
        "pretendard".into(),
        Arc::new(FontData::from_static(include_bytes!("../../assets/fonts/Pretendard-Regular.ttf"))),
    );
    fonts.font_data.insert(
        "pretendard-semibold".into(),
        Arc::new(FontData::from_static(include_bytes!("../../assets/fonts/Pretendard-SemiBold.ttf"))),
    );
    // 한자·일본어 등 Pretendard에 없는 글자는 시스템 글꼴로 보완 (파일 이름에 나올 수 있다).
    let system = [r"C:\Windows\Fonts\malgun.ttf", "/System/Library/Fonts/AppleSDGothicNeo.ttc"]
        .iter()
        .find_map(|p| std::fs::read(p).ok());
    if let Some(bytes) = system {
        fonts.font_data.insert("system".into(), Arc::new(FontData::from_owned(bytes)));
    }
    let has_system = fonts.font_data.contains_key("system");
    let defaults = fonts.families.get(&FontFamily::Proportional).cloned().unwrap_or_default();

    let mut proportional = vec!["pretendard".to_string()];
    let mut bold_family = vec!["pretendard-semibold".to_string()];
    if has_system {
        proportional.push("system".into());
        bold_family.push("system".into());
    }
    proportional.extend(defaults.iter().cloned());
    bold_family.extend(defaults.iter().cloned());
    fonts.families.insert(FontFamily::Proportional, proportional);
    fonts.families.insert(FontFamily::Name(BOLD.into()), bold_family);
    let mono = fonts.families.entry(FontFamily::Monospace).or_default();
    mono.push("pretendard".into());
    if has_system {
        mono.push("system".into());
    }
    ctx.set_fonts(fonts);
}
