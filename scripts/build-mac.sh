#!/bin/sh
# macOS 앱 번들(Apple Silicon + Intel 유니버설)을 만들어 dist/에 둔다.
set -eu
cd "$(dirname "$0")/.."
export PATH=/opt/homebrew/opt/rustup/bin:$PATH

VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
cargo build --release --target aarch64-apple-darwin
cargo build --release --target x86_64-apple-darwin

APP="dist/폴더 잠금.app"
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS"
# 실행 파일 이름은 영문이어야 한다 (한글이면 winit이 메뉴를 만들다 죽는다).
lipo -create -output "$APP/Contents/MacOS/FolderLock" \
  target/aarch64-apple-darwin/release/FolderLock \
  target/x86_64-apple-darwin/release/FolderLock

cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleExecutable</key><string>FolderLock</string>
    <key>CFBundleIdentifier</key><string>com.jmmi.folderlock</string>
    <key>CFBundleName</key><string>폴더 잠금</string>
    <key>CFBundleDisplayName</key><string>폴더 잠금</string>
    <key>CFBundlePackageType</key><string>APPL</string>
    <key>CFBundleShortVersionString</key><string>$VERSION</string>
    <key>CFBundleVersion</key><string>$VERSION</string>
    <key>LSMinimumSystemVersion</key><string>11.0</string>
    <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
PLIST

codesign --force --deep -s - "$APP"
rm -f dist/FolderLock-mac.zip
ditto -c -k --keepParent "$APP" dist/FolderLock-mac.zip
echo "완료: $APP, dist/FolderLock-mac.zip"
