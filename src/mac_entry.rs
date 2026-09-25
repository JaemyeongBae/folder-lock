//! macOS: 잠긴 폴더에 넣는 `잠금 해제.app`. 설치된 앱을 그 폴더의 잠금 해제 창으로 여는 작은 스크립트 앱이다.
//! 내용이 항상 같아야 해시로 "앱이 넣은 것"을 알아볼 수 있으므로 고정 문자열로 만든다.

use std::fs;
use std::io;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

const INFO_PLIST: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
<key>CFBundleExecutable</key><string>unlock</string>
<key>CFBundleIdentifier</key><string>com.jmmi.folderlock.unlock</string>
<key>CFBundleName</key><string>잠금 해제</string>
<key>CFBundlePackageType</key><string>APPL</string>
<key>CFBundleVersion</key><string>1</string>
<key>LSUIElement</key><true/>
</dict>
</plist>
"#;

const SCRIPT: &str = r#"#!/bin/sh
# 폴더 잠금: 이 폴더의 잠금 해제 창을 연다.
DIR="$(cd "$(dirname "$0")/../../.." && pwd)"
if ! open -n -b com.jmmi.folderlock --args --unlock-folder "$DIR"; then
  osascript -e 'display alert "폴더 잠금 앱이 필요합니다" message "이 Mac에 폴더 잠금 앱을 설치한 뒤 다시 열어 주세요."'
fi
"#;

fn write_if_changed(path: &Path, content: &str, mode: u32) -> io::Result<()> {
    if fs::read_to_string(path).ok().as_deref() != Some(content) {
        fs::write(path, content)?;
    }
    fs::set_permissions(path, fs::Permissions::from_mode(mode))
}

pub fn stage(cfg_dir: &Path) -> io::Result<PathBuf> {
    let app = cfg_dir.join("entry").join(crate::meta::UNLOCK_EXE_NAME);
    let macos = app.join("Contents/MacOS");
    fs::create_dir_all(&macos)?;
    write_if_changed(&app.join("Contents/Info.plist"), INFO_PLIST, 0o644)?;
    write_if_changed(&macos.join("unlock"), SCRIPT, 0o755)?;
    Ok(app)
}
