# 폴더 잠금 (FolderLock)

Windows에서 등록한 폴더의 내용을 비밀번호로 숨기는 단일 exe 앱.
파일 내용은 바꾸지 않고, 항목을 숨김 보관 폴더로 옮긴 뒤 목록 보기·삭제를 막는다.
주변 사람의 우연한 열람을 막는 용도다. 설계와 안전 규칙은 `docs/설계안.md` 참고.

## 사용
1. `FolderLock.exe` 실행 → [＋ 폴더 추가] → 폴더 선택 → 비밀번호 설정
2. [잠그기] → 폴더 안에는 `잠금 해제.exe`만 보임
3. 풀 때: 앱에서 [잠금 해제], 또는 폴더 안 `잠금 해제.exe` 실행 후 비밀번호 입력

## 비밀번호를 잊었을 때 (수동 복구)
파일은 암호화되지 않았으므로 관리자 명령 프롬프트에서 직접 되돌릴 수 있다.

```
icacls "D:\내 폴더\.folderlock" /remove:d *S-1-1-0
icacls "D:\내 폴더\.folderlock\data" /remove:d *S-1-1-0
```

그다음 `D:\내 폴더\.folderlock\data` 안의 항목을 `D:\내 폴더`로 옮기면 된다.

## 개발
Rust는 rustup 툴체인을 쓴다 (Homebrew `rust`가 아님).

```sh
export PATH=/opt/homebrew/opt/rustup/bin:$PATH
cargo test                                                    # 엔진 결함 주입 테스트 (Mac)
cargo run                                                     # Mac에서 화면 확인
cargo build --release --target x86_64-pc-windows-gnu          # Windows exe (mingw-w64 필요)
cp target/x86_64-pc-windows-gnu/release/FolderLock.exe dist/
```

- 설정·로그: `%APPDATA%\FolderLock\` (`config.json`, `log.txt`)
- Windows 실기 테스트 항목: `docs/윈도우-테스트.md`
- Mac에서는 실행 파일 이름에 한글·공백이 있으면 창 라이브러리가 죽는다. 그래서 잠긴 폴더 안 exe 모드는 `--unlock-folder <폴더>` 인자로 대신 확인한다.
