//! 직접 그리는 아이콘과 버튼. 글꼴에 기호가 없어 깨지는 일이 없도록 아이콘은 모두 도형으로 그린다.

use std::f32::consts::PI;

use eframe::egui::{
    self, Color32, CursorIcon, Pos2, Rect, Response, Sense, Stroke, StrokeKind, Ui, Vec2, pos2, vec2,
};

use super::theme::{self, Palette, tint};

#[derive(Clone, Copy)]
pub enum Icon {
    Lock,
    Unlock,
    Folder,
    More,
    Plus,
    Close,
    Info,
    Warning,
    Refresh,
    Check,
}

fn arc(center: Pos2, r: f32, from: f32, to: f32) -> Vec<Pos2> {
    let n = 24;
    (0..=n)
        .map(|i| {
            let a = from + (to - from) * i as f32 / n as f32;
            center + vec2(a.cos(), a.sin()) * r
        })
        .collect()
}

/// `rect` 안에 아이콘을 그린다. 선 굵기는 크기에 비례한다.
pub fn paint_icon(painter: &egui::Painter, rect: Rect, icon: Icon, color: Color32) {
    let s = rect.width().min(rect.height());
    let c = rect.center();
    let at = |x: f32, y: f32| pos2(c.x + (x - 0.5) * s, c.y + (y - 0.5) * s);
    let stroke = Stroke::new((s * 0.09).max(1.4), color);
    match icon {
        Icon::Lock | Icon::Unlock => {
            let body = Rect::from_min_max(at(0.2, 0.46), at(0.8, 0.9));
            painter.rect_filled(body, s * 0.1, color);
            let r = s * 0.19;
            let top = at(0.5, 0.34);
            let (left_bottom, right_bottom) = if matches!(icon, Icon::Lock) { (0.46, 0.46) } else { (0.46, 0.3) };
            let mut pts = vec![pos2(top.x - r, at(0.0, left_bottom).y)];
            pts.extend(arc(top, r, PI, 2.0 * PI));
            pts.push(pos2(top.x + r, at(0.0, right_bottom).y));
            let pts: Vec<Pos2> = if matches!(icon, Icon::Unlock) {
                pts.into_iter().map(|p| p + vec2(0.0, -s * 0.08)).collect()
            } else {
                pts
            };
            painter.add(egui::Shape::line(pts, stroke));
            // 열쇠 구멍
            let bg = if color.r() as u32 + color.g() as u32 + color.b() as u32 > 380 { Color32::BLACK } else { Color32::WHITE };
            painter.circle_filled(at(0.5, 0.64), s * 0.055, tint(bg, 150));
            painter.line_segment([at(0.5, 0.66), at(0.5, 0.76)], Stroke::new(s * 0.06, tint(bg, 150)));
        }
        Icon::Folder => {
            let tab = Rect::from_min_max(at(0.1, 0.18), at(0.46, 0.34));
            painter.rect_filled(tab, s * 0.06, color);
            let body = Rect::from_min_max(at(0.1, 0.26), at(0.9, 0.84));
            painter.rect_filled(body, s * 0.08, color);
        }
        Icon::More => {
            for x in [0.25, 0.5, 0.75] {
                painter.circle_filled(at(x, 0.5), s * 0.075, color);
            }
        }
        Icon::Plus => {
            painter.line_segment([at(0.5, 0.2), at(0.5, 0.8)], stroke);
            painter.line_segment([at(0.2, 0.5), at(0.8, 0.5)], stroke);
        }
        Icon::Close => {
            painter.line_segment([at(0.25, 0.25), at(0.75, 0.75)], stroke);
            painter.line_segment([at(0.75, 0.25), at(0.25, 0.75)], stroke);
        }
        Icon::Info => {
            painter.circle_stroke(c, s * 0.42, stroke);
            painter.circle_filled(at(0.5, 0.3), s * 0.06, color);
            painter.line_segment([at(0.5, 0.44), at(0.5, 0.72)], stroke);
        }
        Icon::Warning => {
            let tri = vec![at(0.5, 0.1), at(0.93, 0.86), at(0.07, 0.86), at(0.5, 0.1)];
            painter.add(egui::Shape::line(tri, stroke));
            painter.line_segment([at(0.5, 0.38), at(0.5, 0.6)], stroke);
            painter.circle_filled(at(0.5, 0.72), s * 0.055, color);
        }
        Icon::Refresh => {
            let r = s * 0.3;
            let (from, to) = (-PI * 0.3, PI * 1.3);
            painter.add(egui::Shape::line(arc(c, r, from, to), stroke));
            // 끝점에서 진행 방향으로 향하는 삼각형 화살촉
            let end = c + vec2(to.cos(), to.sin()) * r;
            let forward = vec2(-to.sin(), to.cos());
            let normal = vec2(to.cos(), to.sin());
            let h = s * 0.2;
            painter.add(egui::Shape::convex_polygon(
                vec![end + forward * h, end + normal * h * 0.75, end - normal * h * 0.75],
                color,
                Stroke::NONE,
            ));
        }
        Icon::Check => {
            painter.add(egui::Shape::line(vec![at(0.2, 0.52), at(0.42, 0.72), at(0.8, 0.3)], stroke));
        }
    }
}

/// 아이콘을 줄 안에 그린다.
pub fn icon(ui: &mut Ui, icon: Icon, size: f32, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
    paint_icon(ui.painter(), rect, icon, color);
}

#[derive(Clone, Copy, PartialEq)]
pub enum Kind {
    Primary,
    Secondary,
    Danger,
}

/// 색이 채워진 버튼. 아이콘은 선택.
pub fn button(ui: &mut Ui, kind: Kind, icon: Option<Icon>, label: &str) -> Response {
    let p = Palette::of(ui);
    let (fg, fill, hover, press) = match kind {
        Kind::Primary => (Color32::WHITE, p.accent, p.accent_hover, p.accent_press),
        Kind::Danger => (Color32::WHITE, p.danger, tint(p.danger, 220), tint(p.danger, 190)),
        Kind::Secondary => (p.text, p.subtle, p.subtle_hover, p.subtle_hover),
    };
    let font = if kind == Kind::Secondary { theme::regular(14.0) } else { theme::bold(14.0) };
    let galley = ui.painter().layout_no_wrap(label.to_string(), font, fg);
    let icon_w = if icon.is_some() { 22.0 } else { 0.0 };
    let size = vec2(galley.size().x + icon_w + 30.0, 36.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    if ui.is_rect_visible(rect) {
        let color = if resp.is_pointer_button_down_on() {
            press
        } else if resp.hovered() {
            hover
        } else {
            fill
        };
        ui.painter().rect_filled(rect, 9.0, color);
        let mut x = rect.left() + 15.0;
        if let Some(i) = icon {
            let ir = Rect::from_center_size(pos2(x + 8.0, rect.center().y), Vec2::splat(16.0));
            paint_icon(ui.painter(), ir, i, fg);
            x += icon_w;
        }
        ui.painter().galley(pos2(x, rect.center().y - galley.size().y / 2.0), galley, fg);
    }
    resp.on_hover_cursor(CursorIcon::PointingHand)
}

/// 아이콘만 있는 네모 버튼 (마우스를 올리면 배경이 생김).
pub fn icon_button(ui: &mut Ui, icon: Icon, tooltip: &str) -> Response {
    let p = Palette::of(ui);
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(34.0), Sense::click());
    if ui.is_rect_visible(rect) {
        if resp.hovered() || resp.is_pointer_button_down_on() {
            ui.painter().rect_filled(rect, 9.0, p.subtle_hover);
        }
        let color = if resp.hovered() { p.text } else { p.muted };
        paint_icon(ui.painter(), Rect::from_center_size(rect.center(), Vec2::splat(16.0)), icon, color);
    }
    resp.on_hover_text(tooltip).on_hover_cursor(CursorIcon::PointingHand)
}

/// 상태 배지: 점 + 글자, 연한 색 배경.
pub fn pill(ui: &mut Ui, text: &str, color: Color32) {
    let galley = ui.painter().layout_no_wrap(text.to_string(), theme::bold(12.5), color);
    let size = vec2(galley.size().x + 30.0, 26.0);
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    ui.painter().rect_filled(rect, 13.0, tint(color, 30));
    ui.painter().circle_filled(pos2(rect.left() + 13.0, rect.center().y), 3.5, color);
    ui.painter().galley(pos2(rect.left() + 21.0, rect.center().y - galley.size().y / 2.0), galley, color);
}

/// 둥근 네모 안에 아이콘 (폴더 카드 왼쪽).
pub fn tile(ui: &mut Ui, icon: Icon, color: Color32, size: f32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
    ui.painter().rect_filled(rect, size * 0.28, tint(color, 34));
    paint_icon(ui.painter(), Rect::from_center_size(rect.center(), Vec2::splat(size * 0.46)), icon, color);
}

/// 팝업 메뉴 항목.
pub fn menu_item(ui: &mut Ui, label: &str, danger: bool) -> bool {
    let p = Palette::of(ui);
    let color = if danger { p.danger } else { p.text };
    let galley = ui.painter().layout_no_wrap(label.to_string(), theme::regular(14.0), color);
    let width = ui.available_width().max(galley.size().x + 24.0);
    let (rect, resp) = ui.allocate_exact_size(vec2(width, 34.0), Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(rect, 7.0, if danger { tint(p.danger, 26) } else { p.subtle });
    }
    ui.painter().galley(pos2(rect.left() + 12.0, rect.center().y - galley.size().y / 2.0), galley, color);
    resp.on_hover_cursor(CursorIcon::PointingHand).clicked()
}

/// 넓고 여백이 있는 입력칸.
pub fn text_input(ui: &mut Ui, value: &mut String, hint: &str, password: bool) -> Response {
    let p = Palette::of(ui);
    let edit = egui::TextEdit::singleline(value)
        .password(password)
        .hint_text(egui::RichText::new(hint).color(p.muted))
        .font(theme::regular(15.0))
        .margin(egui::Margin::symmetric(12, 10))
        .desired_width(f32::INFINITY);
    let resp = ui.add(edit);
    let stroke = if resp.has_focus() { Stroke::new(1.5, p.accent) } else { Stroke::new(1.0, p.border) };
    ui.painter().rect_stroke(resp.rect, 9.0, stroke, StrokeKind::Inside);
    resp
}
