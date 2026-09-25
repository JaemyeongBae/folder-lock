#![cfg_attr(windows, windows_subsystem = "windows")]

use std::path::{Path, PathBuf};
use std::sync::Arc;

use eframe::egui;
use folderlock::config::{self, Config, FolderEntry};
use folderlock::engine::{Engine, Error, LockReport, RestoreReport, Status};
use folderlock::guard;
use folderlock::meta::{LockState, VAULT_DIR};
use folderlock::password::{self, MIN_LEN};
use folderlock::platform::OsPlatform;

fn main() -> eframe::Result {
    let cfg_dir = config::config_dir();
    {
        let dir = cfg_dir.clone();
        std::panic::set_hook(Box::new(move |info| {
            config::append_log(&dir, &format!("비정상 종료: {info}"));
        }));
    }

    let target = unlock_target();
    #[cfg(windows)]
    if let Some(folder) = &target {
        // 잠긴 폴더 안의 exe가 실행 중이면 해제 후 그 exe를 지울 수 없으므로 임시 폴더 사본으로 다시 실행한다.
        if !relaunched() && relaunch_from_temp(folder) {
            return Ok(());
        }
    }

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("폴더 잠금")
            .with_inner_size([640.0, 480.0])
            .with_min_inner_size([480.0, 320.0]),
        ..Default::default()
    };
    eframe::run_native(
        "폴더 잠금",
        options,
        Box::new(move |cc| {
            setup_fonts(&cc.egui_ctx);
            Ok(Box::new(App::new(cfg_dir, target)))
        }),
    )
}

/// `--unlock-folder <경로>` 인자, 또는 실행 파일이 잠긴 폴더 안에 있으면 그 폴더.
fn unlock_target() -> Option<PathBuf> {
    let args: Vec<_> = std::env::args_os().collect();
    if let Some(i) = args.iter().position(|a| a == "--unlock-folder") {
        return args.get(i + 1).map(PathBuf::from);
    }
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    dir.join(VAULT_DIR).exists().then(|| dir.to_path_buf())
}

#[cfg(windows)]
fn relaunched() -> bool {
    std::env::args_os().any(|a| a == "--unlock-folder")
}

#[cfg(windows)]
fn relaunch_from_temp(folder: &Path) -> bool {
    let run = || -> std::io::Result<()> {
        let exe = std::env::current_exe()?;
        let dir = std::env::temp_dir().join("FolderLock");
        std::fs::create_dir_all(&dir)?;
        // 예전 사본 정리 (실행 중인 것은 지워지지 않으므로 오류 무시).
        if let Ok(entries) = std::fs::read_dir(&dir) {
            for e in entries.flatten() {
                let name = e.file_name().to_string_lossy().into_owned();
                if name.starts_with("FolderLock-") && name.ends_with(".exe") {
                    let _ = std::fs::remove_file(e.path());
                }
            }
        }
        let copy = dir.join(format!("FolderLock-{}.exe", std::process::id()));
        std::fs::copy(&exe, &copy)?;
        std::process::Command::new(&copy).arg("--unlock-folder").arg(folder).spawn()?;
        Ok(())
    };
    run().is_ok()
}

fn setup_fonts(ctx: &egui::Context) {
    let candidates = [
        r"C:\Windows\Fonts\malgun.ttf",
        "/System/Library/Fonts/AppleSDGothicNeo.ttc",
        "/Library/Fonts/Arial Unicode.ttf",
    ];
    for path in candidates {
        if let Ok(bytes) = std::fs::read(path) {
            let mut fonts = egui::FontDefinitions::default();
            fonts.font_data.insert("korean".into(), Arc::new(egui::FontData::from_owned(bytes)));
            fonts.families.entry(egui::FontFamily::Proportional).or_default().insert(0, "korean".into());
            fonts.families.entry(egui::FontFamily::Monospace).or_default().push("korean".into());
            ctx.set_fonts(fonts);
            break;
        }
    }
    ctx.set_zoom_factor(1.15);
}

fn open_in_file_manager(path: &Path) {
    #[cfg(windows)]
    let _ = std::process::Command::new("explorer").arg(path).spawn();
    #[cfg(target_os = "macos")]
    let _ = std::process::Command::new("open").arg(path).spawn();
}

// ---------- 앱 상태 ----------

struct Notice {
    error: bool,
    text: String,
}

enum Dialog {
    Add { path: PathBuf, pw1: String, pw2: String, error: Option<String> },
    Unlock { path: PathBuf, pw: String, error: Option<String>, open_after: bool },
    ChangePw { path: PathBuf, old: String, new1: String, new2: String, error: Option<String> },
    Remove { path: PathBuf },
    Emergency { path: PathBuf, confirm: String },
}

enum Outcome {
    Keep,
    Cancel,
    Submit,
}

enum Action {
    Add,
    Lock(PathBuf),
    Unlock(PathBuf),
    ChangePw(PathBuf),
    Remove(PathBuf),
    Resume(PathBuf),
    Emergency(PathBuf),
    Open(PathBuf),
    Refresh,
}

struct App {
    engine: Engine<OsPlatform>,
    cfg_dir: PathBuf,
    cfg: Config,
    statuses: Vec<Result<Status, String>>,
    dialog: Option<Dialog>,
    notices: Vec<Notice>,
}

impl App {
    fn new(cfg_dir: PathBuf, target: Option<PathBuf>) -> Self {
        let log_dir = cfg_dir.clone();
        let rules = guard::default_rules(&[cfg_dir.clone()]);
        let engine = Engine::new(OsPlatform, rules).with_logger(move |m| config::append_log(&log_dir, m));
        let mut notices = Vec::new();
        let cfg = match Config::load(&cfg_dir) {
            Ok(c) => c,
            Err(e) => {
                // 망가진 설정을 덮어쓰지 않도록 옆으로 치워 둔다. 잠긴 폴더는 폴더 안 정보로 풀 수 있다.
                let backup = cfg_dir.join(format!("config.broken-{}.json", std::process::id()));
                let _ = std::fs::rename(cfg_dir.join("config.json"), &backup);
                notices.push(Notice { error: true, text: format!("설정 파일을 읽지 못해 새로 시작합니다 ({e}). 이전 파일: {}", backup.display()) });
                Config::default()
            }
        };
        let mut app = App { engine, cfg_dir, cfg, statuses: Vec::new(), dialog: None, notices };
        app.resume_all();
        app.refresh();

        if let Some(folder) = target {
            match app.engine.status(&folder) {
                Ok(Status::Unlocked) => {}
                Ok(_) => {
                    app.dialog = Some(Dialog::Unlock { path: folder, pw: String::new(), error: None, open_after: true });
                }
                Err(e) => app.notices.push(Notice { error: true, text: e.to_string() }),
            }
        }
        app
    }

    fn exe(&self) -> Option<PathBuf> {
        std::env::current_exe().ok()
    }

    /// 중단된 작업을 끝내고, 잠긴 폴더의 숨김·차단을 다시 확인한다.
    fn resume_all(&mut self) {
        let exe = self.exe();
        for entry in self.cfg.folders.clone() {
            let before = self.engine.status(&entry.path).ok();
            match self.engine.resume(&entry.path, exe.as_deref()) {
                Ok(after) => {
                    if let Some(Status::Interrupted(state)) = before {
                        let what = if state == LockState::Locking { "잠그기" } else { "잠금 해제" };
                        let result = if after == Status::Locked { "잠김" } else { "열림" };
                        self.notices.push(Notice { error: false, text: format!("중단됐던 {what}를 마저 끝냈습니다 ({result}): {}", entry.path.display()) });
                    }
                }
                Err(Error::Broken) => {}
                Err(e) => self.notices.push(Notice { error: true, text: format!("{}: {e}", entry.path.display()) }),
            }
        }
    }

    fn refresh(&mut self) {
        self.statuses = self
            .cfg
            .folders
            .iter()
            .map(|f| {
                if !f.path.exists() {
                    return Err("폴더를 찾을 수 없음".into());
                }
                self.engine.status(&f.path).map_err(|e| e.to_string())
            })
            .collect();
    }

    fn save_cfg(&mut self) {
        if let Err(e) = self.cfg.save(&self.cfg_dir) {
            self.notices.push(Notice { error: true, text: format!("설정 저장 실패: {e}") });
        }
    }

    fn info(&mut self, text: impl Into<String>) {
        self.notices.push(Notice { error: false, text: text.into() });
    }

    fn error(&mut self, text: impl Into<String>) {
        self.notices.push(Notice { error: true, text: text.into() });
    }

    fn entry_hash(&self, path: &Path) -> Option<String> {
        self.cfg.find(path).map(|i| self.cfg.folders[i].password_hash.clone())
    }

    // ---------- 동작 ----------

    fn handle(&mut self, action: Action) {
        match action {
            Action::Add => self.pick_folder(),
            Action::Lock(path) => {
                let Some(hash) = self.entry_hash(&path) else { return };
                let exe = self.exe();
                match self.engine.lock(&path, &hash, exe.as_deref()) {
                    Ok(report) => self.info(lock_notice(&path, &report)),
                    Err(e) => self.error(format!("잠그지 못했습니다. 파일은 그대로입니다.\n{e}")),
                }
            }
            Action::Unlock(path) => {
                self.dialog = Some(Dialog::Unlock { path, pw: String::new(), error: None, open_after: false });
            }
            Action::ChangePw(path) => {
                self.dialog = Some(Dialog::ChangePw { path, old: String::new(), new1: String::new(), new2: String::new(), error: None });
            }
            Action::Remove(path) => self.dialog = Some(Dialog::Remove { path }),
            Action::Emergency(path) => self.dialog = Some(Dialog::Emergency { path, confirm: String::new() }),
            Action::Resume(path) => {
                let exe = self.exe();
                match self.engine.resume(&path, exe.as_deref()) {
                    Ok(_) => self.info("이어서 마쳤습니다."),
                    Err(e) => self.error(e.to_string()),
                }
            }
            Action::Open(path) => open_in_file_manager(&path),
            Action::Refresh => {}
        }
        self.refresh();
    }

    fn pick_folder(&mut self) {
        let Some(path) = rfd::FileDialog::new().set_title("잠글 폴더 선택").pick_folder() else { return };
        if let Some(existing) = self.cfg.folders.iter().find(|f| guard::overlaps(&f.path, &path)) {
            let msg = format!("이미 등록된 폴더와 겹칩니다: {}", existing.path.display());
            self.error(msg);
            return;
        }
        match self.engine.status(&path) {
            Ok(Status::Unlocked) => {}
            Ok(Status::Locked) => {
                // 다른 PC·계정에서 잠근 폴더: 풀면서 등록한다.
                self.dialog = Some(Dialog::Unlock { path, pw: String::new(), error: None, open_after: false });
                return;
            }
            Ok(_) => {
                self.error("이전 작업이 중단된 폴더입니다. 폴더 안의 '잠금 해제.exe'로 먼저 정리해 주세요.");
                return;
            }
            Err(e) => {
                self.error(e.to_string());
                return;
            }
        }
        if let Err(msg) = guard::check(&path, &self.engine.rules) {
            self.error(msg);
            return;
        }
        self.dialog = Some(Dialog::Add { path, pw1: String::new(), pw2: String::new(), error: None });
    }

    fn submit(&mut self, dialog: Dialog) {
        match dialog {
            Dialog::Add { path, pw1, pw2, .. } => {
                if let Some(error) = check_new_password(&pw1, &pw2) {
                    self.dialog = Some(Dialog::Add { path, pw1: String::new(), pw2: String::new(), error: Some(error) });
                    return;
                }
                self.cfg.folders.push(FolderEntry { path: path.clone(), password_hash: password::hash_password(&pw1) });
                self.save_cfg();
                self.info(format!("등록했습니다: {}\n[잠그기]를 누르면 잠깁니다.", path.display()));
            }
            Dialog::Unlock { path, pw, open_after, .. } => {
                let fallback = self.entry_hash(&path);
                match self.engine.unlock(&path, &pw, fallback.as_deref()) {
                    Ok(report) => {
                        // 확인된 비밀번호로 등록 정보를 맞춘다 (다른 곳에서 바꾼 경우 포함).
                        let hash = password::hash_password(&pw);
                        match self.cfg.find(&path) {
                            Some(i) => self.cfg.folders[i].password_hash = hash,
                            None => self.cfg.folders.push(FolderEntry { path: path.clone(), password_hash: hash }),
                        }
                        self.save_cfg();
                        self.info(restore_notice(&path, &report));
                        if open_after {
                            open_in_file_manager(&path);
                        }
                    }
                    Err(Error::WrongPassword) => {
                        self.dialog = Some(Dialog::Unlock { path, pw: String::new(), error: Some("비밀번호가 틀렸습니다.".into()), open_after });
                    }
                    Err(e) => self.error(format!("잠금 해제를 마치지 못했습니다.\n{e}")),
                }
            }
            Dialog::ChangePw { path, old, new1, new2, .. } => {
                let locked = matches!(self.engine.status(&path), Ok(Status::Locked));
                let current = if locked { self.engine.read_meta(&path).map(|m| m.password_hash) } else { None }
                    .or_else(|| self.entry_hash(&path));
                let retry = |error: &str| Dialog::ChangePw { path: path.clone(), old: String::new(), new1: String::new(), new2: String::new(), error: Some(error.into()) };
                if !current.is_some_and(|h| password::verify_password(&old, &h)) {
                    self.dialog = Some(retry("현재 비밀번호가 틀렸습니다."));
                    return;
                }
                if let Some(error) = check_new_password(&new1, &new2) {
                    self.dialog = Some(retry(&error));
                    return;
                }
                let hash = password::hash_password(&new1);
                if let Err(e) = self.engine.set_password_hash(&path, &hash) {
                    self.error(format!("비밀번호를 바꾸지 못했습니다. 기존 비밀번호가 그대로 유효합니다.\n{e}"));
                    return;
                }
                if let Some(i) = self.cfg.find(&path) {
                    self.cfg.folders[i].password_hash = hash;
                }
                self.save_cfg();
                self.info("비밀번호를 바꿨습니다.");
            }
            Dialog::Remove { path } => {
                if let Some(i) = self.cfg.find(&path) {
                    self.cfg.folders.remove(i);
                    self.save_cfg();
                    self.info(format!("목록에서 뺐습니다 (폴더와 파일은 그대로): {}", path.display()));
                }
            }
            Dialog::Emergency { path, confirm } => {
                if confirm.trim() != "복원" {
                    self.dialog = Some(Dialog::Emergency { path, confirm });
                    return;
                }
                match self.engine.emergency_restore(&path) {
                    Ok(report) => self.info(restore_notice(&path, &report)),
                    Err(e) => self.error(e.to_string()),
                }
            }
        }
        self.refresh();
    }

    // ---------- 화면 ----------

    fn draw_main(&mut self, ui: &mut egui::Ui, actions: &mut Vec<Action>) {
        ui.horizontal(|ui| {
            ui.heading("폴더 잠금");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("새로 고침").clicked() {
                    actions.push(Action::Refresh);
                }
                if ui.button("＋ 폴더 추가").clicked() {
                    actions.push(Action::Add);
                }
            });
        });
        ui.add_space(4.0);

        let mut dismiss = None;
        for (i, n) in self.notices.iter().enumerate() {
            let color = if n.error { egui::Color32::from_rgb(200, 60, 60) } else { egui::Color32::from_rgb(40, 130, 80) };
            egui::Frame::group(ui.style()).stroke(egui::Stroke::new(1.0, color)).show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.add(egui::Label::new(egui::RichText::new(&n.text).color(color)).wrap());
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::TOP), |ui| {
                        if ui.small_button("닫기").clicked() {
                            dismiss = Some(i);
                        }
                    });
                });
            });
        }
        if let Some(i) = dismiss {
            self.notices.remove(i);
        }
        ui.separator();

        if self.cfg.folders.is_empty() {
            ui.add_space(20.0);
            ui.vertical_centered(|ui| {
                ui.label("등록된 폴더가 없습니다.");
                ui.label("[＋ 폴더 추가]로 숨길 폴더를 등록하세요.");
            });
        }

        egui::ScrollArea::vertical().auto_shrink([false, true]).max_height(ui.available_height() - 60.0).show(ui, |ui| {
            for (entry, status) in self.cfg.folders.iter().zip(&self.statuses) {
                egui::Frame::group(ui.style()).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    let path = entry.path.clone();
                    ui.horizontal(|ui| {
                        let (label, color) = status_label(status);
                        ui.label(egui::RichText::new(label).strong().color(color));
                        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                        ui.add(egui::Label::new(egui::RichText::new(name).strong()).truncate());
                    });
                    ui.add(egui::Label::new(egui::RichText::new(path.display().to_string()).small().weak()).truncate());
                    ui.horizontal(|ui| match status {
                        Ok(Status::Unlocked) => {
                            if ui.button("잠그기").clicked() {
                                actions.push(Action::Lock(path.clone()));
                            }
                            if ui.button("열기").clicked() {
                                actions.push(Action::Open(path.clone()));
                            }
                            if ui.button("비밀번호 변경").clicked() {
                                actions.push(Action::ChangePw(path.clone()));
                            }
                            if ui.button("목록에서 빼기").clicked() {
                                actions.push(Action::Remove(path.clone()));
                            }
                        }
                        Ok(Status::Locked) => {
                            if ui.button("잠금 해제").clicked() {
                                actions.push(Action::Unlock(path.clone()));
                            }
                            if ui.button("비밀번호 변경").clicked() {
                                actions.push(Action::ChangePw(path.clone()));
                            }
                        }
                        Ok(Status::Interrupted(_)) | Ok(Status::Stale) => {
                            if ui.button("이어서 마치기").clicked() {
                                actions.push(Action::Resume(path.clone()));
                            }
                        }
                        Ok(Status::Broken) => {
                            if ui.button("잠금 해제").clicked() {
                                actions.push(Action::Unlock(path.clone()));
                            }
                            if ui.button("비상 복원").clicked() {
                                actions.push(Action::Emergency(path.clone()));
                            }
                        }
                        Err(_) => {
                            if ui.button("목록에서 빼기").clicked() {
                                actions.push(Action::Remove(path.clone()));
                            }
                        }
                    });
                });
            }
        });

        ui.with_layout(egui::Layout::bottom_up(egui::Align::LEFT), |ui| {
            ui.add(
                egui::Label::new(
                    egui::RichText::new(
                        "파일 내용은 바꾸지 않습니다. 폴더 안 항목을 숨김 보관 폴더로 옮기고 열람을 막는 방식입니다. \
                         주변 사람의 우연한 열람을 막는 용도이며, 관리자 권한이 있는 사람은 풀 수 있습니다.",
                    )
                    .small()
                    .weak(),
                )
                .wrap(),
            );
        });
    }
}

fn status_label(status: &Result<Status, String>) -> (String, egui::Color32) {
    let gray = egui::Color32::GRAY;
    match status {
        Ok(Status::Locked) => ("잠김".into(), egui::Color32::from_rgb(40, 130, 80)),
        Ok(Status::Unlocked) => ("열림".into(), gray),
        Ok(Status::Interrupted(_)) => ("중단됨".into(), egui::Color32::from_rgb(210, 140, 30)),
        Ok(Status::Stale) => ("정리 필요".into(), egui::Color32::from_rgb(210, 140, 30)),
        Ok(Status::Broken) => ("상태 정보 손상".into(), egui::Color32::from_rgb(200, 60, 60)),
        Err(e) => (e.clone(), egui::Color32::from_rgb(200, 60, 60)),
    }
}

fn check_new_password(a: &str, b: &str) -> Option<String> {
    if a.chars().count() < MIN_LEN {
        return Some(format!("비밀번호는 {MIN_LEN}자 이상이어야 합니다."));
    }
    if a != b {
        return Some("두 비밀번호가 다릅니다.".into());
    }
    None
}

fn lock_notice(path: &Path, r: &LockReport) -> String {
    let mut s = format!("잠갔습니다 ({}개 항목): {}", r.moved, path.display());
    for w in &r.warnings {
        s.push_str(&format!("\n{w}"));
    }
    s
}

fn restore_notice(path: &Path, r: &RestoreReport) -> String {
    let mut s = format!("잠금을 해제했습니다 ({}개 항목): {}", r.restored, path.display());
    for (from, to) in &r.renamed {
        s.push_str(&format!("\n같은 이름이 있어 '{from}'을(를) '{to}'(으)로 복원했습니다."));
    }
    for w in &r.warnings {
        s.push_str(&format!("\n{w}"));
    }
    s
}

fn folder_label(ui: &mut egui::Ui, path: &Path) {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    ui.add(egui::Label::new(egui::RichText::new(name).strong()).truncate());
    ui.add(egui::Label::new(egui::RichText::new(path.display().to_string()).small().weak()).truncate());
    ui.add_space(4.0);
}

fn password_field(ui: &mut egui::Ui, value: &mut String, hint: &str) -> egui::Response {
    let r = ui.add(egui::TextEdit::singleline(value).password(true).hint_text(hint).desired_width(280.0));
    if ui.memory(|m| m.focused().is_none()) {
        r.request_focus();
    }
    r
}

fn draw_dialog(ctx: &egui::Context, dialog: &mut Dialog) -> Outcome {
    let mut outcome = Outcome::Keep;
    let modal = egui::Modal::new(egui::Id::new("dialog")).show(ctx, |ui| {
        ui.set_width(360.0);
        let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
        let error = |ui: &mut egui::Ui, e: &Option<String>| {
            if let Some(e) = e {
                ui.colored_label(egui::Color32::from_rgb(200, 60, 60), e);
            }
        };
        let (title, ok_label) = match dialog {
            Dialog::Add { .. } => ("폴더 등록", "등록"),
            Dialog::Unlock { .. } => ("잠금 해제", "해제"),
            Dialog::ChangePw { .. } => ("비밀번호 변경", "변경"),
            Dialog::Remove { .. } => ("목록에서 빼기", "빼기"),
            Dialog::Emergency { .. } => ("비상 복원", "복원"),
        };
        ui.heading(title);
        ui.add_space(6.0);
        match dialog {
            Dialog::Add { path, pw1, pw2, error: e } => {
                folder_label(ui, path);
                ui.label("이 폴더에 쓸 비밀번호를 정하세요.");
                password_field(ui, pw1, "비밀번호");
                password_field(ui, pw2, "비밀번호 확인");
                error(ui, e);
            }
            Dialog::Unlock { path, pw, error: e, .. } => {
                folder_label(ui, path);
                password_field(ui, pw, "비밀번호");
                error(ui, e);
            }
            Dialog::ChangePw { path, old, new1, new2, error: e } => {
                folder_label(ui, path);
                password_field(ui, old, "현재 비밀번호");
                password_field(ui, new1, "새 비밀번호");
                password_field(ui, new2, "새 비밀번호 확인");
                error(ui, e);
            }
            Dialog::Remove { path } => {
                folder_label(ui, path);
                ui.label("목록에서만 빠지고 폴더와 파일은 그대로 남습니다.");
            }
            Dialog::Emergency { path, confirm } => {
                folder_label(ui, path);
                ui.label("상태 정보가 손상된 폴더를 비밀번호 없이 원래대로 되돌립니다.\n계속하려면 아래에 '복원'이라고 입력하세요.");
                ui.add(egui::TextEdit::singleline(confirm).desired_width(280.0));
            }
        }
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if ui.button(ok_label).clicked() || enter {
                outcome = Outcome::Submit;
            }
            if ui.button("취소").clicked() {
                outcome = Outcome::Cancel;
            }
        });
    });
    if modal.should_close() && matches!(outcome, Outcome::Keep) {
        outcome = Outcome::Cancel;
    }
    outcome
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let mut actions = Vec::new();
        egui::CentralPanel::default().show(ui, |ui| self.draw_main(ui, &mut actions));
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
}
