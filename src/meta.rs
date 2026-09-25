//! 잠긴 폴더 안에 남기는 상태 파일(meta.json)과 디렉터리 배치.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// 잠긴 폴더 안의 숨김 보관 디렉터리 이름.
pub const VAULT_DIR: &str = ".folderlock";
/// 보관 디렉터리 안에서 사용자 항목이 옮겨지는 곳.
pub const DATA_DIR: &str = "data";
pub const META_FILE: &str = "meta.json";
pub const META_TMP_FILE: &str = "meta.json.tmp";
/// 잠긴 폴더에 복사해 두는 잠금 해제 실행 파일 이름.
pub const UNLOCK_EXE_NAME: &str = "잠금 해제.exe";

pub const META_VERSION: u32 = 1;

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LockState {
    /// 항목을 보관 디렉터리로 옮기는 중.
    Locking,
    Locked,
    /// 항목을 원래 위치로 되돌리는 중. 비밀번호 확인은 이미 끝난 상태.
    Unlocking,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct UnlockExe {
    pub name: String,
    /// 우리가 복사한 파일인지 확인한 뒤에만 지우기 위해 기록한다.
    pub sha256: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Meta {
    pub version: u32,
    pub state: LockState,
    pub password_hash: String,
    #[serde(default)]
    pub unlock_exe: Option<UnlockExe>,
    pub updated_at: String,
}

/// 한 폴더에 대한 경로 모음.
#[derive(Clone, Debug)]
pub struct Layout {
    pub root: PathBuf,
    pub vault: PathBuf,
    pub data: PathBuf,
    pub meta: PathBuf,
    pub meta_tmp: PathBuf,
}

impl Layout {
    pub fn new(root: &Path) -> Self {
        let vault = root.join(VAULT_DIR);
        Layout {
            root: root.to_path_buf(),
            data: vault.join(DATA_DIR),
            meta: vault.join(META_FILE),
            meta_tmp: vault.join(META_TMP_FILE),
            vault,
        }
    }
}

pub fn now_string() -> String {
    humantime::format_rfc3339_seconds(std::time::SystemTime::now()).to_string()
}
