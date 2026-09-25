//! 디스크를 바꾸는 모든 작업은 이 트레이트를 거친다.
//! 테스트는 이 트레이트를 감싸 특정 순번의 작업에서 실패·강제 종료를 흉내 낸다.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::Path;

#[cfg(unix)]
mod unix;
#[cfg(unix)]
use unix as os;
#[cfg(windows)]
mod windows;
#[cfg(windows)]
use self::windows as os;

pub use os::{is_reparse_point, volume_supports_acl};

pub trait Platform {
    fn create_dir(&self, path: &Path) -> io::Result<()>;
    /// 비어 있는 디렉터리만 지운다.
    fn remove_dir(&self, path: &Path) -> io::Result<()>;
    fn remove_file(&self, path: &Path) -> io::Result<()>;
    /// `tmp`에 끝까지 쓰고 디스크에 내린 뒤 `path`로 교체한다.
    fn write_atomic(&self, path: &Path, tmp: &Path, data: &[u8]) -> io::Result<()>;
    /// 대상이 이미 있으면 실패한다.
    fn copy_file_new(&self, from: &Path, to: &Path) -> io::Result<()>;
    /// 같은 볼륨 안에서 이름만 바꾼다. 대상이 있으면 덮어쓰지 않고 실패한다.
    fn move_no_replace(&self, from: &Path, to: &Path) -> io::Result<()>;
    fn hide(&self, path: &Path) -> io::Result<()>;
    /// 목록 보기·삭제를 막는다. 이미 적용돼 있으면 아무것도 하지 않는다.
    fn protect(&self, path: &Path) -> io::Result<()>;
    fn unprotect(&self, path: &Path) -> io::Result<()>;
}

pub struct OsPlatform;

impl Platform for OsPlatform {
    fn create_dir(&self, path: &Path) -> io::Result<()> {
        fs::create_dir(path)
    }

    fn remove_dir(&self, path: &Path) -> io::Result<()> {
        fs::remove_dir(path)
    }

    fn remove_file(&self, path: &Path) -> io::Result<()> {
        fs::remove_file(path)
    }

    fn write_atomic(&self, path: &Path, tmp: &Path, data: &[u8]) -> io::Result<()> {
        {
            let mut f = OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .open(tmp)?;
            f.write_all(data)?;
            f.sync_all()?;
        }
        fs::rename(tmp, path)?;
        if let Some(parent) = path.parent() {
            os::sync_dir(parent)?;
        }
        Ok(())
    }

    fn copy_file_new(&self, from: &Path, to: &Path) -> io::Result<()> {
        let mut src = File::open(from)?;
        let mut dst = OpenOptions::new().write(true).create_new(true).open(to)?;
        let result = io::copy(&mut src, &mut dst).and_then(|_| dst.sync_all());
        if result.is_err() {
            drop(dst);
            // 방금 우리가 만든 불완전한 복사본만 지운다.
            let _ = fs::remove_file(to);
        }
        result
    }

    fn move_no_replace(&self, from: &Path, to: &Path) -> io::Result<()> {
        os::move_no_replace(from, to)
    }

    fn hide(&self, path: &Path) -> io::Result<()> {
        os::hide(path)
    }

    fn protect(&self, path: &Path) -> io::Result<()> {
        os::protect(path)
    }

    fn unprotect(&self, path: &Path) -> io::Result<()> {
        os::unprotect(path)
    }
}
