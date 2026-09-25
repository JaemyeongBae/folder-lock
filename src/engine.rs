//! 잠금 엔진.
//!
//! 안전 원칙
//! 1. 사용자 항목은 같은 볼륨 안에서 "이름 바꾸기"로만 옮긴다. 복사·암호화·덮어쓰기를 하지 않는다.
//! 2. 옮기는 단위는 최상위 항목 하나(파일 또는 폴더 통째)이며, 각 이동은 원자적이다.
//!    따라서 어느 순간 중단돼도 모든 항목은 원래 자리 또는 보관 폴더 중 한 곳에 온전히 있다.
//! 3. 지우는 것은 우리가 만든 것뿐이다: meta.json(.tmp), 빈 디렉터리, 해시가 일치하는 잠금 해제 exe.
//! 4. 상태는 meta.json에 먼저 기록하고 움직인다. 중단되면 다음 실행 때 이어서 끝낸다.

use std::ffi::{OsStr, OsString};
use std::fmt;
use std::fs;
use std::io::{self, Read};
use std::path::Path;

use sha2::{Digest, Sha256};

use crate::guard::{self, Rules};
use crate::meta::{
    now_string, Layout, LockState, Meta, UnlockExe, META_VERSION, UNLOCK_EXE_NAME, VAULT_DIR,
};
use crate::password;
use crate::platform::Platform;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Unlocked,
    Locked,
    /// 잠그기/풀기 도중 중단됨. `resume`으로 이어서 끝낸다.
    Interrupted(LockState),
    /// 보관 폴더가 비어 있고 상태 파일이 없음. 정리만 하면 된다.
    Stale,
    /// 보관 폴더에 항목이 있는데 상태 파일이 없음. `emergency_restore`로만 되돌린다.
    Broken,
}

#[derive(Debug)]
pub enum Error {
    Guard(String),
    AlreadyLocked,
    NotLocked,
    WrongPassword,
    NoPasswordInfo,
    Broken,
    Io { context: String, source: io::Error },
    /// 일부 항목을 되돌리지 못함. 해당 항목은 보관 폴더에 그대로 있으며 다시 시도하면 이어서 진행된다.
    Incomplete { failed: Vec<(String, io::Error)> },
    /// 잠그다 실패해 되돌리려 했지만 되돌리기도 끝나지 않음. 파일은 모두 원래 자리나 보관 폴더에 있다.
    RollbackFailed { cause: Box<Error>, rollback: Box<Error> },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Guard(m) => write!(f, "{m}"),
            Error::AlreadyLocked => write!(f, "이미 잠겨 있거나 정리가 필요한 폴더입니다."),
            Error::NotLocked => write!(f, "잠겨 있지 않은 폴더입니다."),
            Error::WrongPassword => write!(f, "비밀번호가 틀렸습니다."),
            Error::NoPasswordInfo => write!(f, "비밀번호 정보를 찾을 수 없습니다. 비상 복원을 사용하세요."),
            Error::Broken => write!(f, "상태 파일이 없어 비상 복원이 필요합니다."),
            Error::Io { context, source } => write!(f, "{context}: {source}"),
            Error::Incomplete { failed } => {
                write!(f, "{}개 항목을 되돌리지 못했습니다 (파일은 안전하게 보관 중이며 다시 시도하면 이어서 진행됩니다):", failed.len())?;
                for (name, e) in failed.iter().take(5) {
                    write!(f, "\n - {name}: {e}")?;
                }
                Ok(())
            }
            Error::RollbackFailed { cause, rollback } => write!(
                f,
                "잠금에 실패해 되돌리던 중 멈췄습니다. 파일은 모두 안전하며, 프로그램을 다시 열면 이어서 복구합니다.\n원인: {cause}\n되돌리기 오류: {rollback}"
            ),
        }
    }
}

impl std::error::Error for Error {}

fn ctx<T>(r: io::Result<T>, context: impl FnOnce() -> String) -> Result<T, Error> {
    r.map_err(|source| Error::Io { context: context(), source })
}

#[derive(Debug, Default)]
pub struct LockReport {
    pub moved: usize,
    pub warnings: Vec<String>,
}

#[derive(Debug, Default)]
pub struct RestoreReport {
    pub restored: usize,
    /// 이름이 겹쳐 바꿔서 복원한 항목: (원래 이름, 바뀐 이름)
    pub renamed: Vec<(String, String)>,
    pub warnings: Vec<String>,
}

pub fn sha256_file(path: &Path) -> io::Result<String> {
    let mut f = fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hasher.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

fn exists(p: &Path) -> bool {
    fs::symlink_metadata(p).is_ok()
}

fn lossy(name: &OsStr) -> String {
    name.to_string_lossy().into_owned()
}

fn sorted_names(dir: &Path) -> io::Result<Vec<OsString>> {
    let mut names = Vec::new();
    for entry in fs::read_dir(dir)? {
        names.push(entry?.file_name());
    }
    names.sort();
    Ok(names)
}

/// "보고서.docx" → "보고서 (복원 2).docx", 폴더는 "사진 (복원 2)".
fn conflict_name(name: &OsStr, n: usize, is_dir: bool) -> OsString {
    let suffix = format!(" (복원 {n})");
    let path = Path::new(name);
    match (is_dir, path.file_stem(), path.extension()) {
        (false, Some(stem), Some(ext)) => {
            let mut s = stem.to_os_string();
            s.push(&suffix);
            s.push(".");
            s.push(ext);
            s
        }
        _ => {
            let mut s = name.to_os_string();
            s.push(&suffix);
            s
        }
    }
}

pub struct Engine<P: Platform> {
    pub platform: P,
    pub rules: Rules,
    logger: Box<dyn Fn(&str) + Send + Sync>,
}

impl<P: Platform> Engine<P> {
    pub fn new(platform: P, rules: Rules) -> Self {
        Engine { platform, rules, logger: Box::new(|_| {}) }
    }

    pub fn with_logger(mut self, logger: impl Fn(&str) + Send + Sync + 'static) -> Self {
        self.logger = Box::new(logger);
        self
    }

    fn log(&self, msg: impl AsRef<str>) {
        (self.logger)(msg.as_ref());
    }

    // ---------- 상태 ----------

    pub fn read_meta(&self, root: &Path) -> Option<Meta> {
        let l = Layout::new(root);
        let parse = |p: &Path| -> Option<Meta> {
            let bytes = fs::read(p).ok()?;
            serde_json::from_slice(&bytes).ok()
        };
        // meta.json은 원자적으로 교체되므로 있으면 그것이 최신이다.
        // 없을 때만 첫 기록 도중 멈춘 임시 파일을 본다 (디스크에 내린 뒤 교체하므로 읽히면 완전하다).
        if exists(&l.meta) { parse(&l.meta) } else { parse(&l.meta_tmp) }
    }

    pub fn status(&self, root: &Path) -> Result<Status, Error> {
        let l = Layout::new(root);
        match fs::symlink_metadata(&l.vault) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Status::Unlocked),
            Err(e) => return Err(Error::Io { context: "보관 폴더 확인".into(), source: e }),
            Ok(m) if !m.is_dir() => return Ok(Status::Unlocked),
            Ok(_) => {}
        }
        if let Some(meta) = self.read_meta(root) {
            return Ok(match meta.state {
                LockState::Locked => Status::Locked,
                s => Status::Interrupted(s),
            });
        }
        let data_empty = match fs::read_dir(&l.data) {
            Ok(mut it) => it.next().is_none(),
            Err(e) if e.kind() == io::ErrorKind::NotFound => true,
            Err(_) => false,
        };
        Ok(if data_empty { Status::Stale } else { Status::Broken })
    }

    // ---------- 잠그기 ----------

    fn write_meta(&self, l: &Layout, meta: &mut Meta) -> Result<(), Error> {
        meta.updated_at = now_string();
        let bytes = serde_json::to_vec_pretty(meta).expect("meta 직렬화");
        ctx(self.platform.write_atomic(&l.meta, &l.meta_tmp, &bytes), || "상태 파일 기록".into())
    }

    /// 폴더를 잠근다. `unlock_exe_src`가 있으면 잠긴 폴더 안에 잠금 해제 실행 파일을 복사해 둔다.
    pub fn lock(&self, root: &Path, password_hash: &str, unlock_exe_src: Option<&Path>) -> Result<LockReport, Error> {
        guard::check(root, &self.rules).map_err(Error::Guard)?;
        let l = Layout::new(root);
        match self.status(root)? {
            Status::Unlocked => {}
            Status::Stale => {
                self.cleanup_stale(&l)?;
                if exists(&l.vault) {
                    return Err(Error::AlreadyLocked);
                }
            }
            _ => return Err(Error::AlreadyLocked),
        }
        let unlock_exe = match unlock_exe_src {
            Some(src) => Some(UnlockExe {
                name: UNLOCK_EXE_NAME.into(),
                sha256: ctx(sha256_file(src), || "잠금 해제 실행 파일 읽기".into())?,
            }),
            None => None,
        };
        let mut meta = Meta {
            version: META_VERSION,
            state: LockState::Locking,
            password_hash: password_hash.into(),
            unlock_exe,
            updated_at: String::new(),
        };
        self.log(format!("잠금 시작: {}", root.display()));
        ctx(self.platform.create_dir(&l.vault), || "보관 폴더 만들기".into())?;
        if let Err(e) = self.write_meta(&l, &mut meta) {
            let _ = self.cleanup_stale(&l);
            return Err(e);
        }
        match self.finish_lock(&l, &mut meta, unlock_exe_src) {
            Ok(report) => {
                self.log(format!("잠금 완료: {} ({}개 항목)", root.display(), report.moved));
                Ok(report)
            }
            Err(e) => Err(self.rollback(&l, e)),
        }
    }

    fn pending_items(&self, l: &Layout, meta: &Meta) -> Result<Vec<OsString>, Error> {
        let names = ctx(sorted_names(&l.root), || "폴더 목록 읽기".into())?;
        Ok(names
            .into_iter()
            .filter(|name| name != VAULT_DIR)
            .filter(|name| match &meta.unlock_exe {
                // 우리가 복사한 잠금 해제 exe는 옮기지 않는다.
                Some(exe) if name == exe.name.as_str() => {
                    sha256_file(&l.root.join(name)).ok().as_deref() != Some(exe.sha256.as_str())
                }
                _ => true,
            })
            .collect())
    }

    fn finish_lock(&self, l: &Layout, meta: &mut Meta, unlock_exe_src: Option<&Path>) -> Result<LockReport, Error> {
        if !exists(&l.data) {
            ctx(self.platform.create_dir(&l.data), || "보관 폴더 만들기".into())?;
        }
        let mut report = LockReport::default();
        for name in self.pending_items(l, meta)? {
            ctx(self.platform.move_no_replace(&l.root.join(&name), &l.data.join(&name)), || {
                format!("'{}' 옮기기 실패 (다른 프로그램이 사용 중일 수 있습니다)", lossy(&name))
            })?;
            self.log(format!("  보관: {}", lossy(&name)));
            report.moved += 1;
        }
        if let (Some(src), Some(exe)) = (unlock_exe_src, &meta.unlock_exe) {
            let dst = l.root.join(&exe.name);
            if !exists(&dst) {
                if let Err(e) = self.platform.copy_file_new(src, &dst) {
                    report.warnings.push(format!("잠금 해제 실행 파일을 폴더에 넣지 못했습니다: {e}"));
                }
            }
        }
        meta.state = LockState::Locked;
        self.write_meta(l, meta)?;
        self.apply_protection(l)?;
        Ok(report)
    }

    fn apply_protection(&self, l: &Layout) -> Result<(), Error> {
        ctx(self.platform.hide(&l.vault), || "보관 폴더 숨기기".into())?;
        ctx(self.platform.protect(&l.data), || "보관 폴더 접근 차단".into())?;
        ctx(self.platform.protect(&l.vault), || "보관 폴더 접근 차단".into())?;
        Ok(())
    }

    fn rollback(&self, l: &Layout, cause: Error) -> Error {
        self.log(format!("잠금 실패, 되돌리는 중: {cause}"));
        match self.restore_all(l) {
            Ok(_) => {
                self.log("되돌리기 완료");
                cause
            }
            Err(e) => {
                self.log(format!("되돌리기 실패: {e}"));
                Error::RollbackFailed { cause: Box::new(cause), rollback: Box::new(e) }
            }
        }
    }

    // ---------- 풀기 ----------

    /// 비밀번호를 확인하고 모든 항목을 원래 자리로 되돌린다.
    /// 상태 파일이 없을 때에만 `fallback_hash`(앱 설정에 저장된 해시)를 쓴다.
    pub fn unlock(&self, root: &Path, password: &str, fallback_hash: Option<&str>) -> Result<RestoreReport, Error> {
        match self.status(root)? {
            Status::Unlocked | Status::Stale => return Err(Error::NotLocked),
            _ => {}
        }
        let hash = self
            .read_meta(root)
            .map(|m| m.password_hash)
            .or_else(|| fallback_hash.map(String::from))
            .ok_or(Error::NoPasswordInfo)?;
        if !password::verify_password(password, &hash) {
            self.log(format!("비밀번호 틀림: {}", root.display()));
            return Err(Error::WrongPassword);
        }
        self.log(format!("잠금 해제 시작: {}", root.display()));
        let report = self.restore_all(&Layout::new(root))?;
        self.log(format!("잠금 해제 완료: {} ({}개 항목)", root.display(), report.restored));
        Ok(report)
    }

    /// 상태 파일이 없거나 손상된 경우, 비밀번호 없이 모든 항목을 되돌린다.
    pub fn emergency_restore(&self, root: &Path) -> Result<RestoreReport, Error> {
        self.log(format!("비상 복원: {}", root.display()));
        self.restore_all(&Layout::new(root))
    }

    fn restore_all(&self, l: &Layout) -> Result<RestoreReport, Error> {
        let mut report = RestoreReport::default();
        for dir in [&l.vault, &l.data] {
            if exists(dir) {
                ctx(self.platform.unprotect(dir), || "보관 폴더 접근 허용".into())?;
            }
        }
        let meta = self.read_meta(&l.root);
        if let Some(mut m) = meta.clone() {
            if m.state != LockState::Unlocking {
                m.state = LockState::Unlocking;
                self.write_meta(l, &mut m)?;
            }
        }

        // 잠금 해제 exe를 먼저 치워야 같은 이름의 사용자 파일이 원래 이름으로 돌아간다.
        if let Some(exe) = meta.as_ref().and_then(|m| m.unlock_exe.clone()) {
            let p = l.root.join(&exe.name);
            match sha256_file(&p) {
                Ok(h) if h == exe.sha256 => {
                    if let Err(e) = self.platform.remove_file(&p) {
                        report.warnings.push(format!("'{}'를 지우지 못했습니다. 직접 지워도 됩니다: {e}", exe.name));
                    }
                }
                Ok(_) => report.warnings.push(format!("'{}'가 처음 넣은 파일과 달라 그대로 두었습니다.", exe.name)),
                Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                Err(e) => report.warnings.push(format!("'{}' 확인 실패: {e}", exe.name)),
            }
        }

        let mut failed = Vec::new();
        if exists(&l.data) {
            for name in ctx(sorted_names(&l.data), || "보관 폴더 목록 읽기".into())? {
                match self.restore_one(l, &name) {
                    Ok(None) => report.restored += 1,
                    Ok(Some(new_name)) => {
                        report.restored += 1;
                        report.renamed.push((lossy(&name), lossy(&new_name)));
                    }
                    Err(e) => failed.push((lossy(&name), e)),
                }
            }
        }
        if !failed.is_empty() {
            return Err(Error::Incomplete { failed });
        }

        for f in [&l.meta, &l.meta_tmp] {
            match self.platform.remove_file(f) {
                Err(e) if e.kind() != io::ErrorKind::NotFound => {
                    return Err(Error::Io { context: "상태 파일 정리".into(), source: e });
                }
                _ => {}
            }
        }
        self.remove_empty_dirs(l, &mut report.warnings);
        Ok(report)
    }

    fn restore_one(&self, l: &Layout, name: &OsStr) -> io::Result<Option<OsString>> {
        let src = l.data.join(name);
        let is_dir = fs::symlink_metadata(&src)?.is_dir();
        for n in 0..1000 {
            let candidate = if n == 0 { name.to_os_string() } else { conflict_name(name, n, is_dir) };
            if candidate == VAULT_DIR {
                continue;
            }
            let dst = l.root.join(&candidate);
            if exists(&dst) {
                continue;
            }
            match self.platform.move_no_replace(&src, &dst) {
                Ok(()) => {
                    self.log(format!("  복원: {}", lossy(&candidate)));
                    return Ok(if n == 0 { None } else { Some(candidate) });
                }
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(e),
            }
        }
        Err(io::Error::other("겹치지 않는 이름을 찾지 못했습니다"))
    }

    fn remove_empty_dirs(&self, l: &Layout, warnings: &mut Vec<String>) {
        for dir in [&l.data, &l.vault] {
            match self.platform.remove_dir(dir) {
                Ok(()) => {}
                Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                Err(e) => warnings.push(format!(
                    "'{}' 폴더에 알 수 없는 파일이 남아 있어 지우지 않았습니다: {e}",
                    dir.display()
                )),
            }
        }
    }

    fn cleanup_stale(&self, l: &Layout) -> Result<(), Error> {
        match self.platform.remove_file(&l.meta_tmp) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => {
                return Err(Error::Io { context: "임시 상태 파일 정리".into(), source: e });
            }
            _ => {}
        }
        let mut warnings = Vec::new();
        self.remove_empty_dirs(l, &mut warnings);
        if let Some(w) = warnings.into_iter().next() {
            return Err(Error::Guard(w));
        }
        Ok(())
    }

    // ---------- 복구·관리 ----------

    /// 중단된 작업을 이어서 끝내고, 잠긴 폴더의 숨김·차단 상태를 다시 확인한다.
    pub fn resume(&self, root: &Path, unlock_exe_src: Option<&Path>) -> Result<Status, Error> {
        let l = Layout::new(root);
        match self.status(root)? {
            Status::Unlocked => Ok(Status::Unlocked),
            Status::Stale => {
                self.cleanup_stale(&l)?;
                self.status(root)
            }
            Status::Locked => {
                self.apply_protection(&l)?;
                Ok(Status::Locked)
            }
            Status::Interrupted(LockState::Locking) => {
                self.log(format!("중단된 잠금 이어서 진행: {}", root.display()));
                let mut meta = self.read_meta(root).ok_or(Error::Broken)?;
                for dir in [&l.vault, &l.data] {
                    if exists(dir) {
                        ctx(self.platform.unprotect(dir), || "보관 폴더 접근 허용".into())?;
                    }
                }
                match self.finish_lock(&l, &mut meta, unlock_exe_src) {
                    Ok(_) => Ok(Status::Locked),
                    Err(e) => Err(self.rollback(&l, e)),
                }
            }
            Status::Interrupted(_) => {
                self.log(format!("중단된 잠금 해제 이어서 진행: {}", root.display()));
                self.restore_all(&l)?;
                Ok(Status::Unlocked)
            }
            Status::Broken => Err(Error::Broken),
        }
    }

    /// 잠긴 폴더의 비밀번호 해시를 바꾼다. 잠겨 있지 않으면 아무것도 하지 않는다.
    pub fn set_password_hash(&self, root: &Path, new_hash: &str) -> Result<(), Error> {
        if self.status(root)? != Status::Locked {
            return Ok(());
        }
        let l = Layout::new(root);
        let mut meta = self.read_meta(root).ok_or(Error::Broken)?;
        meta.password_hash = new_hash.into();
        ctx(self.platform.unprotect(&l.vault), || "보관 폴더 접근 허용".into())?;
        let written = self.write_meta(&l, &mut meta);
        // 기록 성공 여부와 관계없이 차단은 다시 건다.
        self.apply_protection(&l)?;
        written
    }
}
