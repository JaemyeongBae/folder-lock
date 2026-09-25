//! 메인 화면과 대화상자.

use std::path::{Path, PathBuf};

use eframe::egui::{self, Align, Frame, Layout, Margin, RichText, Sense, Stroke, Ui, Vec2};
use folderlock::engine::Status;

use super::theme::{self, Palette, tint};
use super::widgets::{self, Icon, Kind};
use crate::{Action, App, Dialog, Outcome};

#[cfg(windows)]
const OPEN_LABEL: &str = "탐색기에서 열기";
#[cfg(not(windows))]
const OPEN_LABEL: &str = "Finder에서 열기";

impl App {
    pub(crate) fn draw(&mut self, ui: &mut Ui) {
        let p = Palette::of(ui);
        let ctx = ui.ctx().clone();
        let mut actions = Vec::new();

        egui::Panel::bottom(egui::Id::new("footer"))
            .frame(Frame::new().fill(p.bg).inner_margin(Margin::symmetric(28, 14)))
            .show_separator_line(false)
            .show(ui, |ui| footer(ui, &p));
        egui::CentralPanel::default()
            .frame(Frame::new().fill(p.bg).inner_margin(Margin { left: 28, right: 28, top: 24, bottom: 0 }))
            .show(ui, |ui| self.draw_main(ui, &p, &mut actions));

        if let Some(mut dialog) = self.dialog.take() {
            match draw_dialog(&ctx, &mut dialog) {
                Outcome::Keep => self.dialog = Some(dialog),
                Outcome::Cancel => {}
                Outcome::Submit => self.submit(dialog),
            }
        }
        for action in actions {
            self.handle(action);
        }
    }

    fn draw_main(&mut self, ui: &mut Ui, p: &Palette, actions: &mut Vec<Action>) {
        header(ui, p, &self.statuses, actions);
        ui.add_space(18.0);

        // 성공 알림은 잠시 뒤 저절로 사라진다. 오류는 직접 닫을 때까지 남긴다.
        const INFO_TTL: std::time::Duration = std::time::Duration::from_secs(6);
        self.notices.retain(|n| n.error || n.at.elapsed() < INFO_TTL);
        if self.notices.iter().any(|n| !n.error) {
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(500));
        }
        let mut dismiss = None;
        for (i, n) in self.notices.iter().enumerate() {
            if notice(ui, p, n.error, &n.text) {
                dismiss = Some(i);
            }
            ui.add_space(8.0);
        }
        if let Some(i) = dismiss {
            self.notices.remove(i);
        }

        if self.cfg.folders.is_empty() {
            empty_state(ui, p, actions);
            return;
        }
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            for (entry, status) in self.cfg.folders.iter().zip(&self.statuses) {
                folder_card(ui, p, &entry.path, status, actions);
                ui.add_space(10.0);
            }
        });
    }
}

fn header(ui: &mut Ui, p: &Palette, statuses: &[Result<Status, String>], actions: &mut Vec<Action>) {
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(Vec2::splat(46.0), Sense::hover());
        ui.painter().rect_filled(rect, 13.0, p.accent);
        widgets::paint_icon(ui.painter(), rect.shrink(12.0), Icon::Lock, egui::Color32::WHITE);
        ui.add_space(4.0);
        ui.vertical(|ui| {
            ui.add_space(2.0);
            ui.label(RichText::new("폴더 잠금").font(theme::bold(21.0)).color(p.text));
            let locked = statuses.iter().filter(|s| matches!(s, Ok(Status::Locked))).count();
            let summary = if statuses.is_empty() {
                "등록한 폴더를 비밀번호로 숨깁니다".to_string()
            } else {
                format!("폴더 {}개 · 잠김 {}개", statuses.len(), locked)
            };
            ui.label(RichText::new(summary).font(theme::regular(13.0)).color(p.muted));
        });
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if widgets::button(ui, Kind::Primary, Some(Icon::Plus), "폴더 추가").clicked() {
                actions.push(Action::Add);
            }
            if widgets::icon_button(ui, Icon::Refresh, "새로 고침").clicked() {
                actions.push(Action::Refresh);
            }
        });
    });
}

/// 알림 한 개. 닫기를 누르면 true.
fn notice(ui: &mut Ui, p: &Palette, error: bool, text: &str) -> bool {
    let color = if error { p.danger } else { p.success };
    let mut closed = false;
    Frame::new()
        .fill(tint(color, 22))
        .stroke(Stroke::new(1.0, tint(color, 80)))
        .corner_radius(12)
        .inner_margin(Margin { left: 14, right: 6, top: 8, bottom: 8 })
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                widgets::icon(ui, if error { Icon::Warning } else { Icon::Check }, 18.0, color);
                ui.add_space(2.0);
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    closed = widgets::icon_button(ui, Icon::Close, "닫기").clicked();
                    ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                        ui.add(egui::Label::new(RichText::new(text).color(p.text)).wrap());
                    });
                });
            });
        });
    closed
}

fn empty_state(ui: &mut Ui, p: &Palette, actions: &mut Vec<Action>) {
    ui.add_space((ui.available_height() * 0.16).max(12.0));
    ui.vertical_centered(|ui| {
        let (rect, _) = ui.allocate_exact_size(Vec2::splat(80.0), Sense::hover());
        ui.painter().rect_filled(rect, 24.0, tint(p.accent, 30));
        widgets::paint_icon(ui.painter(), rect.shrink(22.0), Icon::Lock, p.accent);
        ui.add_space(16.0);
        ui.label(RichText::new("아직 등록된 폴더가 없어요").font(theme::bold(18.0)).color(p.text));
        ui.add_space(2.0);
        ui.label(RichText::new("숨기고 싶은 폴더를 추가하고 비밀번호를 정하면").color(p.muted));
        ui.label(RichText::new("버튼 하나로 잠그고 풀 수 있어요.").color(p.muted));
        ui.add_space(14.0);
        if widgets::button(ui, Kind::Primary, Some(Icon::Plus), "폴더 추가").clicked() {
            actions.push(Action::Add);
        }
    });
}

fn footer(ui: &mut Ui, p: &Palette) {
    ui.horizontal(|ui| {
        widgets::icon(ui, Icon::Info, 14.0, p.muted);
        ui.add(
            egui::Label::new(
                RichText::new("파일 내용은 그대로 두고 숨김 폴더로 옮겨 잠급니다 · 주변 사람의 우연한 열람을 막는 용도")
                    .font(theme::regular(12.0))
                    .color(p.muted),
            )
            .truncate(),
        );
    });
}

struct Look {
    label: String,
    color: egui::Color32,
    icon: Icon,
}

fn look(p: &Palette, status: &Result<Status, String>) -> Look {
    let (label, color, icon) = match status {
        Ok(Status::Locked) => ("잠김", p.success, Icon::Lock),
        Ok(Status::Unlocked) => ("열림", p.muted, Icon::Folder),
        Ok(Status::Interrupted(_)) => ("중단됨", p.warning, Icon::Warning),
        Ok(Status::Stale) => ("정리 필요", p.warning, Icon::Warning),
        Ok(Status::Broken) => ("손상", p.danger, Icon::Warning),
        Err(e) if e.contains("찾을 수 없") => ("찾을 수 없음", p.danger, Icon::Warning),
        Err(_) => ("오류", p.danger, Icon::Warning),
    };
    Look { label: label.into(), color, icon }
}

fn folder_card(ui: &mut Ui, p: &Palette, path: &Path, status: &Result<Status, String>, actions: &mut Vec<Action>) {
    let look = look(p, status);
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| path.display().to_string());
    let path_buf = path.to_path_buf();

    let (primary, menu): (Option<(Kind, Option<Icon>, &str, Action)>, Vec<(&str, bool, Action)>) = match status {
        Ok(Status::Unlocked) => (
            Some((Kind::Primary, Some(Icon::Lock), "잠그기", Action::Lock(path_buf.clone()))),
            vec![
                (OPEN_LABEL, false, Action::Open(path_buf.clone())),
                ("비밀번호 변경", false, Action::ChangePw(path_buf.clone())),
                ("목록에서 빼기", true, Action::Remove(path_buf.clone())),
            ],
        ),
        Ok(Status::Locked) => (
            Some((Kind::Primary, Some(Icon::Unlock), "잠금 해제", Action::Unlock(path_buf.clone()))),
            vec![("비밀번호 변경", false, Action::ChangePw(path_buf.clone()))],
        ),
        Ok(Status::Interrupted(_)) | Ok(Status::Stale) => {
            (Some((Kind::Primary, None, "이어서 마치기", Action::Resume(path_buf.clone()))), vec![])
        }
        Ok(Status::Broken) => (
            Some((Kind::Primary, Some(Icon::Unlock), "잠금 해제", Action::Unlock(path_buf.clone()))),
            vec![("비상 복원", true, Action::Emergency(path_buf.clone()))],
        ),
        Err(_) => (Some((Kind::Secondary, None, "목록에서 빼기", Action::Remove(path_buf.clone()))), vec![]),
    };

    Frame::new()
        .fill(p.card)
        .stroke(Stroke::new(1.0, p.border))
        .corner_radius(14)
        .inner_margin(Margin::symmetric(16, 14))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                widgets::tile(ui, look.icon, look.color, 46.0);
                ui.add_space(6.0);
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if !menu.is_empty() {
                        let more = widgets::icon_button(ui, Icon::More, "더 보기");
                        egui::Popup::menu(&more).show(|ui| {
                            ui.set_width(190.0);
                            ui.spacing_mut().item_spacing.y = 2.0;
                            for (label, danger, action) in menu {
                                if widgets::menu_item(ui, label, danger) {
                                    actions.push(action);
                                }
                            }
                        });
                    }
                    if let Some((kind, icon, label, action)) = primary {
                        if widgets::button(ui, kind, icon, label).clicked() {
                            actions.push(action);
                        }
                    }
                    ui.add_space(6.0);
                    widgets::pill(ui, &look.label, look.color);
                    ui.add_space(10.0);
                    ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                        ui.vertical(|ui| {
                            ui.spacing_mut().item_spacing.y = 3.0;
                            ui.add(egui::Label::new(RichText::new(&name).font(theme::bold(15.5)).color(p.text)).truncate());
                            let detail = match status {
                                Err(e) if !e.contains("찾을 수 없") => e.clone(),
                                _ => path.display().to_string(),
                            };
                            ui.add(egui::Label::new(RichText::new(detail).font(theme::regular(12.0)).color(p.muted)).truncate());
                        });
                    });
                });
            });
        });
}

// ---------- 대화상자 ----------

fn folder_chip(ui: &mut Ui, p: &Palette, path: &PathBuf) {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    Frame::new().fill(p.subtle).corner_radius(10).inner_margin(Margin::symmetric(12, 10)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            widgets::icon(ui, Icon::Folder, 18.0, p.muted);
            ui.add_space(2.0);
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                ui.add(egui::Label::new(RichText::new(name).font(theme::bold(14.0)).color(p.text)).truncate());
                ui.add(egui::Label::new(RichText::new(path.display().to_string()).font(theme::regular(11.5)).color(p.muted)).truncate());
            });
        });
    });
}

pub(crate) fn draw_dialog(ctx: &egui::Context, dialog: &mut Dialog) -> Outcome {
    let mut outcome = Outcome::Keep;
    let p = if ctx.global_style().visuals.dark_mode { Palette::DARK } else { Palette::LIGHT };
    let frame = Frame::new()
        .fill(p.card)
        .stroke(Stroke::new(1.0, p.border))
        .corner_radius(18)
        .inner_margin(Margin::same(26))
        .shadow(ctx.global_style().visuals.window_shadow);
    let modal = egui::Modal::new(egui::Id::new("dialog")).frame(frame).show(ctx, |ui| {
        ui.set_width(400.0);
        ui.spacing_mut().item_spacing.y = 10.0;
        let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));

        let (title, desc, submit, kind) = match dialog {
            Dialog::Add { .. } => ("폴더 등록", "이 폴더를 잠그고 풀 때 쓸 비밀번호를 정하세요.", "등록", Kind::Primary),
            Dialog::Unlock { .. } => ("잠금 해제", "비밀번호를 입력하면 파일이 원래 자리로 돌아옵니다.", "잠금 해제", Kind::Primary),
            Dialog::ChangePw { .. } => ("비밀번호 변경", "현재 비밀번호를 확인한 뒤 새 비밀번호로 바꿉니다.", "변경", Kind::Primary),
            Dialog::Remove { .. } => ("목록에서 빼기", "목록에서만 빠지고 폴더와 파일은 그대로 남아요.", "빼기", Kind::Primary),
            Dialog::Emergency { .. } => (
                "비상 복원",
                "상태 정보가 손상된 폴더를 비밀번호 없이 원래대로 되돌립니다. 계속하려면 아래에 '복원'이라고 입력하세요.",
                "복원",
                Kind::Danger,
            ),
        };
        ui.label(RichText::new(title).font(theme::bold(19.0)).color(p.text));
        ui.add(egui::Label::new(RichText::new(desc).color(p.muted)).wrap());
        ui.add_space(2.0);

        let mut inputs = Vec::new();
        let error = match dialog {
            Dialog::Add { path, pw1, pw2, error } => {
                folder_chip(ui, &p, path);
                inputs.push(widgets::text_input(ui, pw1, "비밀번호 (4자 이상)", true));
                inputs.push(widgets::text_input(ui, pw2, "비밀번호 확인", true));
                error.clone()
            }
            Dialog::Unlock { path, pw, error, .. } => {
                folder_chip(ui, &p, path);
                inputs.push(widgets::text_input(ui, pw, "비밀번호", true));
                error.clone()
            }
            Dialog::ChangePw { path, old, new1, new2, error } => {
                folder_chip(ui, &p, path);
                inputs.push(widgets::text_input(ui, old, "현재 비밀번호", true));
                inputs.push(widgets::text_input(ui, new1, "새 비밀번호 (4자 이상)", true));
                inputs.push(widgets::text_input(ui, new2, "새 비밀번호 확인", true));
                error.clone()
            }
            Dialog::Remove { path } => {
                folder_chip(ui, &p, path);
                None
            }
            Dialog::Emergency { path, confirm } => {
                folder_chip(ui, &p, path);
                inputs.push(widgets::text_input(ui, confirm, "복원", false));
                None
            }
        };
        if ui.memory(|m| m.focused().is_none()) {
            if let Some(first) = inputs.first() {
                first.request_focus();
            }
        }
        if let Some(e) = error {
            ui.horizontal(|ui| {
                widgets::icon(ui, Icon::Warning, 16.0, p.danger);
                ui.label(RichText::new(e).color(p.danger));
            });
        }

        ui.add_space(6.0);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if widgets::button(ui, kind, None, submit).clicked() || enter {
                outcome = Outcome::Submit;
            }
            if widgets::button(ui, Kind::Secondary, None, "취소").clicked() {
                outcome = Outcome::Cancel;
            }
        });
    });
    if modal.should_close() && matches!(outcome, Outcome::Keep) {
        outcome = Outcome::Cancel;
    }
    outcome
}
