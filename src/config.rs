//! 앱 설정: 등록된 폴더 목록과 각 폴더의 비밀번호 해시.
//! 잠긴 폴더 안의 meta.json이 원본이고, 여기 해시는 상태 파일이 망가졌을 때의 예비용이다.

use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct FolderEntry {
    pub path: PathBuf,
    pub password_hash: String,
}

#[derive(Serialize, Deserialize, Default, Clone, Debug)]
pub struct Config {
    pub folders: Vec<FolderEntry>,
}

pub fn config_dir() -> PathBuf {
    #[cfg(windows)]
    let base = std::env::var_os("APPDATA").map(PathBuf::from);
    #[cfg(target_os = "macos")]
    let base = std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Library/Application Support"));
    #[cfg(all(unix, not(target_os = "macos")))]
    let base = std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config"));
    base.unwrap_or_else(std::env::temp_dir).join("FolderLock")
}

impl Config {
    pub fn load(dir: &Path) -> io::Result<Config> {
        match fs::read(dir.join("config.json")) {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(io::Error::other),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Config::default()),
            Err(e) => Err(e),
        }
    }

    pub fn save(&self, dir: &Path) -> io::Result<()> {
        fs::create_dir_all(dir)?;
        let tmp = dir.join("config.json.tmp");
        {
            let mut f = OpenOptions::new().write(true).create(true).truncate(true).open(&tmp)?;
            f.write_all(&serde_json::to_vec_pretty(self).expect("설정 직렬화"))?;
            f.sync_all()?;
        }
        fs::rename(tmp, dir.join("config.json"))
    }

    pub fn find(&self, path: &Path) -> Option<usize> {
        let n = crate::guard::normalize(path);
        self.folders.iter().position(|f| crate::guard::normalize(&f.path) == n)
    }
}

/// 설정 폴더의 log.txt에 한 줄 덧붙인다. 실패는 무시한다.
pub fn append_log(dir: &Path, line: &str) {
    let _ = fs::create_dir_all(dir);
    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(dir.join("log.txt")) {
        let _ = writeln!(f, "{} {}", crate::meta::now_string(), line);
    }
}
