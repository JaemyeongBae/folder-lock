//! Windows 구현.
//! - 이동: MoveFileExW(플래그 0) — 같은 볼륨 안에서만, 절대 덮어쓰지 않음, 복사+삭제로 대체하지 않음.
//! - 숨김: 숨김+시스템 속성 (탐색기 기본 설정에서 "보호된 운영 체제 파일"로 취급되어 안 보임).
//! - 차단: Everyone에 대해 목록 보기·삭제를 거부하는 ACE 하나를 추가.
//!   소유자는 항상 권한(DACL)을 바꿀 수 있으므로 이 ACE는 언제든 우리가 되돌릴 수 있다.

use std::ffi::c_void;
use std::fs::Metadata;
use std::io;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::fs::MetadataExt;
use std::path::Path;
use std::ptr::null_mut;

use windows_sys::Win32::Foundation::{ERROR_SUCCESS, LocalFree};
use windows_sys::Win32::Security::Authorization::{
    DENY_ACCESS, EXPLICIT_ACCESS_W, GetNamedSecurityInfoW, NO_MULTIPLE_TRUSTEE, SE_FILE_OBJECT,
    SetEntriesInAclW, SetNamedSecurityInfoW, TRUSTEE_IS_SID, TRUSTEE_IS_WELL_KNOWN_GROUP, TRUSTEE_W,
};
use windows_sys::Win32::Security::{
    ACCESS_DENIED_ACE, ACE_HEADER, ACL, ACL_SIZE_INFORMATION, AclSizeInformation,
    CreateWellKnownSid, DACL_SECURITY_INFORMATION, DeleteAce, EqualSid, GetAce,
    GetAclInformation, INHERITED_ACE, NO_INHERITANCE, PSECURITY_DESCRIPTOR, PSID,
    SECURITY_MAX_SID_SIZE, WinWorldSid,
};
use windows_sys::Win32::Storage::FileSystem::{
    DELETE, FILE_ATTRIBUTE_HIDDEN, FILE_ATTRIBUTE_REPARSE_POINT, FILE_ATTRIBUTE_SYSTEM,
    FILE_DELETE_CHILD, FILE_LIST_DIRECTORY, GetFileAttributesW, GetVolumeInformationW,
    GetVolumePathNameW, INVALID_FILE_ATTRIBUTES, MoveFileExW, SetFileAttributesW,
};

/// 우리가 추가하는 거부 권한. 이 값으로 우리 ACE를 알아본다.
const DENY_MASK: u32 = FILE_LIST_DIRECTORY | DELETE | FILE_DELETE_CHILD;
const ACCESS_DENIED_ACE_TYPE: u8 = 1;

fn wide(p: &Path) -> Vec<u16> {
    let s = p.as_os_str();
    let raw: Vec<u16> = s.encode_wide().collect();
    // 긴 경로는 \\?\ 접두어가 있어야 Win32 API가 받아 준다.
    let needs_prefix = raw.len() >= 240
        && p.is_absolute()
        && !raw.starts_with(&['\\' as u16, '\\' as u16]);
    let mut out = Vec::with_capacity(raw.len() + 5);
    if needs_prefix {
        out.extend(r"\\?\".encode_utf16());
    }
    out.extend(raw);
    out.push(0);
    out
}

fn win32_err(code: u32) -> io::Error {
    io::Error::from_raw_os_error(code as i32)
}

pub fn move_no_replace(from: &Path, to: &Path) -> io::Result<()> {
    let f = wide(from);
    let t = wide(to);
    // 플래그 0: MOVEFILE_REPLACE_EXISTING 없음(덮어쓰기 금지), MOVEFILE_COPY_ALLOWED 없음(볼륨 간 복사 금지).
    let ok = unsafe { MoveFileExW(f.as_ptr(), t.as_ptr(), 0) };
    if ok != 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

pub fn sync_dir(_dir: &Path) -> io::Result<()> {
    Ok(())
}

pub fn hide(path: &Path) -> io::Result<()> {
    let w = wide(path);
    let attrs = unsafe { GetFileAttributesW(w.as_ptr()) };
    if attrs == INVALID_FILE_ATTRIBUTES {
        return Err(io::Error::last_os_error());
    }
    let ok = unsafe {
        SetFileAttributesW(w.as_ptr(), attrs | FILE_ATTRIBUTE_HIDDEN | FILE_ATTRIBUTE_SYSTEM)
    };
    if ok != 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

fn everyone_sid() -> io::Result<Vec<u8>> {
    let mut buf = vec![0u8; SECURITY_MAX_SID_SIZE as usize];
    let mut size = SECURITY_MAX_SID_SIZE;
    let ok = unsafe {
        CreateWellKnownSid(WinWorldSid, null_mut(), buf.as_mut_ptr() as PSID, &mut size)
    };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    buf.truncate(size as usize);
    Ok(buf)
}

/// 경로의 DACL과 이를 담은 보안 서술자. 서술자는 drop 때 해제한다.
struct Dacl {
    acl: *mut ACL,
    sd: PSECURITY_DESCRIPTOR,
}

impl Drop for Dacl {
    fn drop(&mut self) {
        if !self.sd.is_null() {
            unsafe { LocalFree(self.sd as _) };
        }
    }
}

fn read_dacl(w: &[u16]) -> io::Result<Dacl> {
    let mut acl: *mut ACL = null_mut();
    let mut sd: PSECURITY_DESCRIPTOR = null_mut();
    let r = unsafe {
        GetNamedSecurityInfoW(
            w.as_ptr(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION,
            null_mut(),
            null_mut(),
            &mut acl,
            null_mut(),
            &mut sd,
        )
    };
    if r != ERROR_SUCCESS {
        return Err(win32_err(r));
    }
    Ok(Dacl { acl, sd })
}

fn write_dacl(w: &[u16], acl: *const ACL) -> io::Result<()> {
    let r = unsafe {
        SetNamedSecurityInfoW(
            w.as_ptr(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION,
            null_mut(),
            null_mut(),
            acl,
            null_mut(),
        )
    };
    if r != ERROR_SUCCESS {
        return Err(win32_err(r));
    }
    Ok(())
}

/// DACL에서 우리 거부 비트를 가진 Everyone 거부 ACE들을 찾는다: (인덱스, ACE 포인터).
unsafe fn our_aces(acl: *mut ACL, sid: &[u8]) -> io::Result<Vec<(u32, *mut ACCESS_DENIED_ACE)>> {
    let mut found = Vec::new();
    if acl.is_null() {
        return Ok(found);
    }
    let mut info = ACL_SIZE_INFORMATION::default();
    let ok = unsafe {
        GetAclInformation(
            acl,
            &mut info as *mut _ as *mut c_void,
            size_of::<ACL_SIZE_INFORMATION>() as u32,
            AclSizeInformation,
        )
    };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    for i in 0..info.AceCount {
        let mut ace: *mut c_void = null_mut();
        if unsafe { GetAce(acl, i, &mut ace) } == 0 {
            return Err(io::Error::last_os_error());
        }
        let header = unsafe { &*(ace as *const ACE_HEADER) };
        if header.AceType != ACCESS_DENIED_ACE_TYPE || (header.AceFlags as u32 & INHERITED_ACE) != 0 {
            continue;
        }
        let denied = ace as *mut ACCESS_DENIED_ACE;
        let mask = unsafe { (*denied).Mask };
        let ace_sid = unsafe { &mut (*denied).SidStart as *mut u32 as PSID };
        let same = unsafe { EqualSid(ace_sid, sid.as_ptr() as PSID) } != 0;
        if same && (mask & DENY_MASK) == DENY_MASK {
            found.push((i, denied));
        }
    }
    Ok(found)
}

pub fn protect(path: &Path) -> io::Result<()> {
    let w = wide(path);
    let sid = everyone_sid()?;
    let current = read_dacl(&w)?;
    if !unsafe { our_aces(current.acl, &sid)? }.is_empty() {
        return Ok(());
    }
    let ea = EXPLICIT_ACCESS_W {
        grfAccessPermissions: DENY_MASK,
        grfAccessMode: DENY_ACCESS,
        grfInheritance: NO_INHERITANCE,
        Trustee: TRUSTEE_W {
            pMultipleTrustee: null_mut(),
            MultipleTrusteeOperation: NO_MULTIPLE_TRUSTEE,
            TrusteeForm: TRUSTEE_IS_SID,
            TrusteeType: TRUSTEE_IS_WELL_KNOWN_GROUP,
            ptstrName: sid.as_ptr() as *mut u16,
        },
    };
    let mut new_acl: *mut ACL = null_mut();
    let r = unsafe { SetEntriesInAclW(1, &ea, current.acl, &mut new_acl) };
    if r != ERROR_SUCCESS {
        return Err(win32_err(r));
    }
    let result = write_dacl(&w, new_acl);
    unsafe { LocalFree(new_acl as _) };
    result
}

pub fn unprotect(path: &Path) -> io::Result<()> {
    let w = wide(path);
    let sid = everyone_sid()?;
    let current = read_dacl(&w)?;
    let aces = unsafe { our_aces(current.acl, &sid)? };
    if aces.is_empty() {
        return Ok(());
    }
    // 뒤에서부터 지워야 앞쪽 인덱스가 바뀌지 않는다.
    for (index, ace) in aces.into_iter().rev() {
        let mask = unsafe { (*ace).Mask };
        if mask == DENY_MASK {
            if unsafe { DeleteAce(current.acl, index) } == 0 {
                return Err(io::Error::last_os_error());
            }
        } else {
            // 기존 거부 ACE와 합쳐진 경우: 우리가 넣은 비트만 뺀다.
            unsafe { (*ace).Mask = mask & !DENY_MASK };
        }
    }
    write_dacl(&w, current.acl)
}

pub fn is_reparse_point(meta: &Metadata) -> bool {
    meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

/// 폴더가 있는 볼륨이 NTFS(또는 ACL을 지원하는 ReFS)인지 확인한다.
pub fn volume_supports_acl(path: &Path) -> io::Result<bool> {
    let w = wide(path);
    let mut root = vec![0u16; 1024];
    if unsafe { GetVolumePathNameW(w.as_ptr(), root.as_mut_ptr(), root.len() as u32) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let mut fs_name = vec![0u16; 64];
    let ok = unsafe {
        GetVolumeInformationW(
            root.as_ptr(),
            null_mut(),
            0,
            null_mut(),
            null_mut(),
            null_mut(),
            fs_name.as_mut_ptr(),
            fs_name.len() as u32,
        )
    };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    let end = fs_name.iter().position(|&c| c == 0).unwrap_or(fs_name.len());
    let name = String::from_utf16_lossy(&fs_name[..end]).to_uppercase();
    Ok(name == "NTFS" || name == "REFS")
}
