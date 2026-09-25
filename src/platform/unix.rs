//! macOS 구현 (Linux는 개발·테스트용).
//! - 이동: renamex_np(RENAME_EXCL) — 같은 볼륨 안에서만, 대상이 있으면 실패.
//! - 숨김: Finder 숨김 플래그(UF_HIDDEN). 보관 폴더 이름도 점으로 시작한다.
//! - 차단: 권한 0o300(목록 보기 불가, 경로로는 접근 가능) + 잠금 플래그(UF_IMMUTABLE,
//!   삭제·이름 바꾸기·항목 추가/삭제 불가). 소유자는 언제든 둘 다 되돌릴 수 있다.

use std::ffi::CString;
use std::fs::{self, File, Metadata, Permissions};
use std::io;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

fn cstr(p: &Path) -> io::Result<CString> {
    CString::new(p.as_os_str().as_bytes()).map_err(io::Error::other)
}

fn check(r: libc::c_int) -> io::Result<()> {
    if r == 0 { Ok(()) } else { Err(io::Error::last_os_error()) }
}

pub fn move_no_replace(from: &Path, to: &Path) -> io::Result<()> {
    let f = cstr(from)?;
    let t = cstr(to)?;
    #[cfg(target_os = "macos")]
    let r = unsafe { libc::renamex_np(f.as_ptr(), t.as_ptr(), libc::RENAME_EXCL) };
    #[cfg(target_os = "linux")]
    let r = unsafe {
        libc::renameat2(libc::AT_FDCWD, f.as_ptr(), libc::AT_FDCWD, t.as_ptr(), libc::RENAME_NOREPLACE)
    };
    check(r)
}

pub fn sync_dir(dir: &Path) -> io::Result<()> {
    File::open(dir)?.sync_all()
}

#[cfg(target_os = "macos")]
fn set_flags(path: &Path, add: u32, remove: u32) -> io::Result<()> {
    use std::os::macos::fs::MetadataExt;
    let current = fs::symlink_metadata(path)?.st_flags();
    let wanted = (current | add) & !remove;
    if wanted == current {
        return Ok(());
    }
    let p = cstr(path)?;
    check(unsafe { libc::chflags(p.as_ptr(), wanted as _) })
}

#[cfg(target_os = "macos")]
pub fn hide(path: &Path) -> io::Result<()> {
    set_flags(path, libc::UF_HIDDEN, 0)
}

#[cfg(not(target_os = "macos"))]
pub fn hide(_path: &Path) -> io::Result<()> {
    Ok(())
}

pub fn protect(path: &Path) -> io::Result<()> {
    // 잠금 플래그가 걸리면 권한도 못 바꾸므로 권한부터.
    #[cfg(target_os = "macos")]
    if is_immutable(path)? {
        return Ok(());
    }
    fs::set_permissions(path, Permissions::from_mode(0o300))?;
    #[cfg(target_os = "macos")]
    set_flags(path, libc::UF_IMMUTABLE, 0)?;
    Ok(())
}

pub fn unprotect(path: &Path) -> io::Result<()> {
    #[cfg(target_os = "macos")]
    set_flags(path, 0, libc::UF_IMMUTABLE)?;
    fs::set_permissions(path, Permissions::from_mode(0o700))
}

#[cfg(target_os = "macos")]
fn is_immutable(path: &Path) -> io::Result<bool> {
    use std::os::macos::fs::MetadataExt;
    Ok(fs::symlink_metadata(path)?.st_flags() & libc::UF_IMMUTABLE != 0)
}

pub fn is_reparse_point(meta: &Metadata) -> bool {
    meta.file_type().is_symlink()
}

/// macOS: 권한·플래그를 제대로 지원하는 APFS/HFS+만 허용한다 (exFAT USB 등은 거부).
#[cfg(target_os = "macos")]
pub fn volume_supports_acl(path: &Path) -> io::Result<bool> {
    let p = cstr(path)?;
    let mut st: libc::statfs = unsafe { std::mem::zeroed() };
    check(unsafe { libc::statfs(p.as_ptr(), &mut st) })?;
    let name: Vec<u8> = st
        .f_fstypename
        .iter()
        .take_while(|&&c| c != 0)
        .map(|&c| c as u8)
        .collect();
    let local = st.f_flags & libc::MNT_LOCAL as u32 != 0;
    Ok(local && matches!(name.as_slice(), b"apfs" | b"hfs"))
}

#[cfg(not(target_os = "macos"))]
pub fn volume_supports_acl(_path: &Path) -> io::Result<bool> {
    Ok(true)
}
