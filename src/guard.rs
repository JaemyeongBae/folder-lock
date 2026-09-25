//! 잠그면 안 되는 폴더를 걸러낸다.

use std::fs;
use std::path::{Path, PathBuf};

use crate::platform;

#[derive(Default, Clone, Debug)]
pub struct Rules {
    /// 이 폴더 자체는 금지 (하위 폴더는 허용). 예: 바탕 화면, 문서.
    pub forbidden_exact: Vec<PathBuf>,
    /// 이 폴더와 그 아래 전부 금지. 예: C:\Windows, OneDrive.
    pub forbidden_tree: Vec<PathBuf>,
    /// 잠글 폴더가 이 경로들을 품고 있으면 금지. 예: 앱 실행 파일, 설정 폴더.
    pub must_not_contain: Vec<PathBuf>,
}

/// 비교용 정규화: 실제 경로로 풀고, \\?\ 접두어와 끝 구분자를 떼고, Windows에선 소문자로.
pub fn normalize(p: &Path) -> String {
    let real = fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    let mut s = real.to_string_lossy().into_owned();
    if let Some(rest) = s.strip_prefix(r"\\?\") {
        if !rest.starts_with("UNC\\") {
            s = rest.to_string();
        }
    }
    while s.len() > 1 && (s.ends_with('/') || s.ends_with('\\')) && !s.ends_with(":\\") {
        s.pop();
    }
    if cfg!(windows) {
        s = s.to_lowercase();
    }
    s
}

/// `inner`가 `outer`와 같거나 그 아래에 있는가.
pub fn is_within(inner: &str, outer: &str) -> bool {
    if inner == outer {
        return true;
    }
    let sep_outer = if outer.ends_with('/') || outer.ends_with('\\') {
        outer.to_string()
    } else if cfg!(windows) {
        format!("{outer}\\")
    } else {
        format!("{outer}/")
    };
    inner.starts_with(&sep_outer)
}

pub fn check(root: &Path, rules: &Rules) -> Result<(), String> {
    if !root.is_absolute() {
        return Err("절대 경로가 아닙니다.".into());
    }
    let meta = fs::symlink_metadata(root).map_err(|e| format!("폴더를 읽을 수 없습니다: {e}"))?;
    if platform::is_reparse_point(&meta) {
        return Err("바로 가기(링크·정션) 폴더는 잠글 수 없습니다.".into());
    }
    if !meta.is_dir() {
        return Err("폴더가 아닙니다.".into());
    }
    let n = normalize(root);
    if n.starts_with(r"\\") {
        return Err("네트워크 폴더는 잠글 수 없습니다.".into());
    }
    let real = PathBuf::from(&n);
    if real.parent().is_none() || n.ends_with(":\\") || n.len() <= 3 {
        return Err("드라이브 전체는 잠글 수 없습니다.".into());
    }
    for p in &rules.forbidden_exact {
        if n == normalize(p) {
            return Err(format!("시스템이 쓰는 폴더라 잠글 수 없습니다: {}", p.display()));
        }
    }
    for p in &rules.forbidden_tree {
        if is_within(&n, &normalize(p)) {
            return Err(format!("이 위치 아래의 폴더는 잠글 수 없습니다: {}", p.display()));
        }
    }
    for p in &rules.must_not_contain {
        if is_within(&normalize(p), &n) {
            return Err(format!(
                "폴더 잠금 프로그램이 쓰는 파일이 들어 있어 잠글 수 없습니다: {}",
                p.display()
            ));
        }
    }
    match platform::volume_supports_acl(root) {
        Ok(true) => {}
        Ok(false) => return Err("NTFS 드라이브의 폴더만 잠글 수 있습니다 (USB 메모리 등은 지원하지 않음).".into()),
        Err(e) => return Err(format!("드라이브 정보를 읽을 수 없습니다: {e}")),
    }
    Ok(())
}

/// 두 폴더가 같거나 한쪽이 다른 쪽 안에 있는가 (중첩 등록 방지).
pub fn overlaps(a: &Path, b: &Path) -> bool {
    let (a, b) = (normalize(a), normalize(b));
    is_within(&a, &b) || is_within(&b, &a)
}

/// 현재 OS의 기본 규칙.
pub fn default_rules(extra_protected: &[PathBuf]) -> Rules {
    let mut rules = Rules {
        must_not_contain: extra_protected.to_vec(),
        ..Default::default()
    };
    if let Ok(exe) = std::env::current_exe() {
        rules.must_not_contain.push(exe);
    }
    let env = |k: &str| std::env::var_os(k).map(PathBuf::from);

    #[cfg(windows)]
    {
        for k in ["SystemRoot", "windir", "ProgramFiles", "ProgramFiles(x86)", "ProgramW6432", "ProgramData", "APPDATA", "LOCALAPPDATA"] {
            if let Some(p) = env(k) {
                rules.forbidden_tree.push(p);
            }
        }
        // 클라우드 동기화 폴더는 동기화 충돌 위험이 있어 막는다.
        for k in ["OneDrive", "OneDriveConsumer", "OneDriveCommercial"] {
            if let Some(p) = env(k) {
                rules.forbidden_tree.push(p);
            }
        }
        if let Some(home) = env("USERPROFILE") {
            rules.forbidden_tree.push(home.join("AppData"));
            for sync in ["Dropbox", "iCloudDrive", "Google Drive", "MYBOX"] {
                rules.forbidden_tree.push(home.join(sync));
            }
            for sub in ["Desktop", "Documents", "Downloads", "Pictures", "Music", "Videos", "Favorites", "Links", "Contacts", "Saved Games", "Searches", "3D Objects"] {
                rules.forbidden_exact.push(home.join(sub));
            }
            rules.forbidden_exact.push(home);
        }
        if let Some(public) = env("PUBLIC") {
            rules.forbidden_exact.push(public);
        }
        rules.forbidden_exact.push(PathBuf::from(r"C:\Users"));
    }

    #[cfg(unix)]
    {
        for p in ["/System", "/Library", "/Applications", "/usr", "/bin", "/sbin", "/etc", "/var", "/private/etc", "/private/var"] {
            rules.forbidden_tree.push(PathBuf::from(p));
        }
        if let Some(home) = env("HOME") {
            rules.forbidden_tree.push(home.join("Library"));
            for sync in ["Dropbox", "Google Drive", "MYBOX", "OneDrive"] {
                rules.forbidden_tree.push(home.join(sync));
            }
            // "데스크탑 및 문서 폴더" iCloud 동기화가 켜져 있으면 그 아래 전부 동기화되므로 막는다.
            let icloud = home.join("Library/Mobile Documents/com~apple~CloudDocs");
            for sub in ["Desktop", "Documents"] {
                if icloud.join(sub).exists() {
                    rules.forbidden_tree.push(home.join(sub));
                }
            }
            for sub in ["Desktop", "Documents", "Downloads", "Pictures", "Music", "Movies"] {
                rules.forbidden_exact.push(home.join(sub));
            }
            rules.forbidden_exact.push(home);
        }
    }
    rules
}
