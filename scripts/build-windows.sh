#!/bin/sh
# Windows exe를 크로스 컴파일해 dist/에 둔다 (brew install mingw-w64 필요).
set -eu
cd "$(dirname "$0")/.."
export PATH=/opt/homebrew/opt/rustup/bin:$PATH
# 빌드한 Mac의 홈 경로(사용자 이름)가 실행 파일에 남지 않게 한다.
export RUSTFLAGS="--remap-path-prefix=$HOME=/build"
cargo build --release --target x86_64-pc-windows-gnu
mkdir -p dist
cp target/x86_64-pc-windows-gnu/release/FolderLock.exe dist/
echo "완료: dist/FolderLock.exe"
