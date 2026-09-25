#![cfg_attr(windows, windows_subsystem = "windows")]

use std::path::{Path, PathBuf};

use eframe::egui;
use folderlock::config::{self, Config, FolderEntry};
use folderlock::engine::{Engine, Error, LockReport, RestoreReport, Status};
use folderlock::guard;
use folderlock::meta::{LockState, VAULT_DIR};
use folderlock::password::{self, MIN_LEN};
use folderlock::platform::OsPlatform;

mod ui;

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
            .with_inner_size([760.0, 560.0])
            .with_min_inner_size([560.0, 420.0]),
        ..Default::default()
    };
    eframe::run_native(
        "폴더 잠금",
        options,
        Box::new(move |cc| {
            ui::theme::install(&cc.egui_ctx);
            // 개발용: FOLDERLOCK_THEME=light|dark 로 테마 고정 (기본은 시스템 설정).
            match std::env::var("FOLDERLOCK_THEME").as_deref() {
                Ok("light") => cc.egui_ctx.set_theme(egui::Theme::Light),
                Ok("dark") => cc.egui_ctx.set_theme(egui::Theme::Dark),
                _ => {}
            }
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
    at: std::time::Instant,
}

impl Notice {
    fn new(error: bool, text: impl Into<String>) -> Self {
        Notice { error, text: text.into(), at: std::time::Instant::now() }
    }
}

/// 알림에 보여 줄 폴더 이름.
fn folder_name(path: &Path) -> String {
    path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| path.display().to_string())
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
                notices.push(Notice::new(true, format!("설정 파일을 읽지 못해 새로 시작해요 ({e}). 이전 파일: {}", backup.display())));
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
                Err(e) => app.notices.push(Notice::new(true, e.to_string())),
            }
        }
        app
    }

    /// 잠긴 폴더에 넣을 잠금 해제 입구의 원본.
    fn exe(&self) -> Option<PathBuf> {
        #[cfg(windows)]
        return std::env::current_exe().ok();
        #[cfg(not(windows))]
        return folderlock::mac_entry::stage(&self.cfg_dir).ok();
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
                        self.notices.push(Notice::new(false, format!("'{}' 폴더의 중단됐던 {what}를 마저 끝냈어요 ({result}).", folder_name(&entry.path))));
                    }
                }
                Err(Error::Broken) => {}
                Err(e) => self.notices.push(Notice::new(true, format!("'{}': {e}", folder_name(&entry.path)))),
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
            self.notices.push(Notice::new(true, format!("설정을 저장하지 못했어요: {e}")));
        }
    }

    fn info(&mut self, text: impl Into<String>) {
        self.notices.push(Notice::new(false, text));
    }

    fn error(&mut self, text: impl Into<String>) {
        self.notices.push(Notice::new(true, text));
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
                    Err(e) => self.error(format!("잠그지 못했어요. 파일은 모두 그대로예요.\n{e}")),
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
                    Ok(_) => self.info(format!("'{}' 폴더의 작업을 마저 끝냈어요.", folder_name(&path))),
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
                self.error("이전 작업이 중단된 폴더예요. 폴더 안의 잠금 해제 입구로 먼저 정리해 주세요.");
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
                self.info(format!("'{}' 폴더를 등록했어요. [잠그기]를 누르면 잠겨요.", folder_name(&path)));
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
                    Err(e) => self.error(format!("잠금 해제를 마치지 못했어요.\n{e}")),
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
                    self.error(format!("비밀번호를 바꾸지 못했어요. 기존 비밀번호가 그대로 유효해요.\n{e}"));
                    return;
                }
                if let Some(i) = self.cfg.find(&path) {
                    self.cfg.folders[i].password_hash = hash;
                }
                self.save_cfg();
                self.info(format!("'{}' 폴더의 비밀번호를 바꿨어요.", folder_name(&path)));
            }
            Dialog::Remove { path } => {
                if let Some(i) = self.cfg.find(&path) {
                    self.cfg.folders.remove(i);
                    self.save_cfg();
                    self.info(format!("'{}' 폴더를 목록에서 뺐어요. 폴더와 파일은 그대로예요.", folder_name(&path)));
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
    let mut s = format!("'{}' 폴더를 잠갔어요 ({}개 항목).", folder_name(path), r.moved);
    for w in &r.warnings {
        s.push_str(&format!("\n{w}"));
    }
    s
}

fn restore_notice(path: &Path, r: &RestoreReport) -> String {
    let mut s = format!("'{}' 폴더의 잠금을 해제했어요 ({}개 항목).", folder_name(path), r.restored);
    for (from, to) in &r.renamed {
        s.push_str(&format!("\n같은 이름이 있어서 '{from}'은(는) '{to}'(으)로 복원했어요."));
    }
    for w in &r.warnings {
        s.push_str(&format!("\n{w}"));
    }
    s
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.draw(ui);
    }
}
