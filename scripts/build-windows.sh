#!/bin/sh
# Windows exe를 크로스 컴파일해 dist/에 둔다 (brew install mingw-w64 필요).
set -eu
cd "$(dirname "$0")/.."
export PATH=/opt/homebrew/opt/rustup/bin:$PATH
cargo build --release --target x86_64-pc-windows-gnu
mkdir -p dist
cp target/x86_64-pc-windows-gnu/release/FolderLock.exe dist/
echo "완료: dist/FolderLock.exe"
