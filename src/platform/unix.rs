//! 개발·테스트용 구현. Windows의 "목록 보기 금지, 경로로는 접근 가능" 동작을
//! 디렉터리 권한 0o300(쓰기+통과만 허용)으로 흉내 낸다.

use std::ffi::CString;
use std::fs::{self, File, Metadata, Permissions};
use std::io;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

fn cstr(p: &Path) -> io::Result<CString> {
    CString::new(p.as_os_str().as_bytes()).map_err(io::Error::other)
}

pub fn move_no_replace(from: &Path, to: &Path) -> io::Result<()> {
    let f = cstr(from)?;
    let t = cstr(to)?;
    #[cfg(target_os = "macos")]
    let r = unsafe { libc::renamex_np(f.as_ptr(), t.as_ptr(), libc::RENAME_EXCL) };
    #[cfg(target_os = "linux")]
    let r = unsafe {
        libc::renameat2(
            libc::AT_FDCWD,
            f.as_ptr(),
            libc::AT_FDCWD,
            t.as_ptr(),
            libc::RENAME_NOREPLACE,
        )
    };
    if r == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

pub fn sync_dir(dir: &Path) -> io::Result<()> {
    File::open(dir)?.sync_all()
}

pub fn hide(_path: &Path) -> io::Result<()> {
    Ok(())
}

pub fn protect(path: &Path) -> io::Result<()> {
    fs::set_permissions(path, Permissions::from_mode(0o300))
}

pub fn unprotect(path: &Path) -> io::Result<()> {
    fs::set_permissions(path, Permissions::from_mode(0o700))
}

pub fn is_reparse_point(meta: &Metadata) -> bool {
    meta.file_type().is_symlink()
}

pub fn volume_supports_acl(_path: &Path) -> io::Result<bool> {
    Ok(true)
}
