#![cfg(unix)]
//! 잠금 엔진 테스트. 핵심 속성:
//! "어느 작업 단계에서 강제 종료되더라도, 다시 실행해 복구하면 모든 파일이 바이트 단위로 원래와 같다."

use std::cell::Cell;
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fs;
use std::io;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use folderlock::engine::{Engine, Error, Status};
use folderlock::guard::Rules;
use folderlock::meta::{LockState, DATA_DIR, UNLOCK_EXE_NAME, VAULT_DIR};
use folderlock::password::hash_password;
use folderlock::platform::{OsPlatform, Platform};

const PW: &str = "correct horse";

// ---------- 결함 주입 플랫폼 ----------

#[derive(Default)]
struct Faulty {
    count: Cell<usize>,
    /// 이 순번부터 모든 작업이 실패 (프로세스 강제 종료와 같은 효과: 이후 디스크 변화 없음)
    crash_at: Option<usize>,
    /// 이 순번의 작업 하나만 실패 (일시적 오류 → 엔진이 되돌리기를 수행)
    fail_once_at: Option<usize>,
    /// 이 이름을 옮기려 하면 실패 (다른 프로그램이 파일을 사용 중인 상황)
    fail_move_of: Option<OsString>,
}

impl Faulty {
    fn crash(at: usize) -> Self {
        Faulty { crash_at: Some(at), ..Default::default() }
    }
    fn step(&self) -> io::Result<()> {
        let n = self.count.get();
        self.count.set(n + 1);
        if self.crash_at.is_some_and(|c| n >= c) {
            return Err(io::Error::other("강제 종료 흉내"));
        }
        if self.fail_once_at == Some(n) {
            return Err(io::Error::other("일시적 오류 흉내"));
        }
        Ok(())
    }
}

impl Platform for Faulty {
    fn create_dir(&self, p: &Path) -> io::Result<()> {
        self.step()?;
        OsPlatform.create_dir(p)
    }
    fn remove_dir(&self, p: &Path) -> io::Result<()> {
        self.step()?;
        OsPlatform.remove_dir(p)
    }
    fn remove_file(&self, p: &Path) -> io::Result<()> {
        self.step()?;
        OsPlatform.remove_file(p)
    }
    fn write_atomic(&self, p: &Path, tmp: &Path, d: &[u8]) -> io::Result<()> {
        self.step()?;
        OsPlatform.write_atomic(p, tmp, d)
    }
    fn copy_file_new(&self, f: &Path, t: &Path) -> io::Result<()> {
        self.step()?;
        OsPlatform.copy_file_new(f, t)
    }
    fn move_no_replace(&self, f: &Path, t: &Path) -> io::Result<()> {
        self.step()?;
        if self.fail_move_of.as_deref().is_some_and(|n| f.file_name() == Some(n)) {
            return Err(io::Error::other("사용 중인 파일 흉내"));
        }
        OsPlatform.move_no_replace(f, t)
    }
    fn hide(&self, p: &Path) -> io::Result<()> {
        self.step()?;
        OsPlatform.hide(p)
    }
    fn protect(&self, p: &Path) -> io::Result<()> {
        self.step()?;
        OsPlatform.protect(p)
    }
    fn unprotect(&self, p: &Path) -> io::Result<()> {
        self.step()?;
        OsPlatform.unprotect(p)
    }
}

// ---------- 도우미 ----------

#[derive(Debug, PartialEq, Eq, Clone)]
enum Entry {
    Dir,
    File(Vec<u8>),
}

type Snapshot = BTreeMap<PathBuf, Entry>;

fn snapshot(root: &Path) -> Snapshot {
    fn walk(base: &Path, dir: &Path, out: &mut Snapshot) {
        for e in fs::read_dir(dir).unwrap() {
            let e = e.unwrap();
            let p = e.path();
            let rel = p.strip_prefix(base).unwrap().to_path_buf();
            if e.file_type().unwrap().is_dir() {
                out.insert(rel, Entry::Dir);
                walk(base, &p, out);
            } else {
                out.insert(rel, Entry::File(fs::read(&p).unwrap()));
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(root, root, &mut out);
    out
}

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

const NAMES: &[&str] = &[
    "문서", "사진 2024", "report.docx", ".hidden", "emoji😀.txt", "a b c", "데이터.csv",
    "archive.tar.gz", "빈 폴더", "README", "메모.txt", "x",
];

fn make_tree(root: &Path, seed: u64) {
    let mut rng = Rng(seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407) | 1);
    fn fill(dir: &Path, depth: u32, rng: &mut Rng) {
        let count = 1 + rng.below(if depth == 0 { 6 } else { 4 });
        for i in 0..count {
            let name = format!("{}{}", NAMES[rng.below(NAMES.len() as u64) as usize], if i > 0 { format!("_{i}") } else { String::new() });
            let p = dir.join(&name);
            if p.exists() {
                continue;
            }
            if depth < 3 && rng.below(3) == 0 {
                fs::create_dir(&p).unwrap();
                if rng.below(4) != 0 {
                    fill(&p, depth + 1, rng);
                }
            } else {
                let size = match rng.below(4) {
                    0 => 0,
                    1 => rng.below(100),
                    _ => rng.below(200_000),
                } as usize;
                let data: Vec<u8> = (0..size).map(|_| rng.next() as u8).collect();
                fs::write(&p, data).unwrap();
            }
        }
    }
    fill(root, 0, &mut rng);
}

struct Fixture {
    _tmp: tempfile::TempDir,
    root: PathBuf,
    exe: PathBuf,
    hash: String,
}

fn fixture(seed: u64) -> Fixture {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("잠글 폴더");
    fs::create_dir(&root).unwrap();
    make_tree(&root, seed);
    let exe = tmp.path().join("app.exe");
    fs::write(&exe, format!("fake exe {seed}").repeat(1000)).unwrap();
    Fixture { hash: hash_password(PW), root, exe, _tmp: tmp }
}

fn engine<P: Platform>(p: P) -> Engine<P> {
    Engine::new(p, Rules::default())
}

/// 테스트용: 보호 권한을 풀어 내부를 볼 수 있게 한다 (엔진의 unprotect와 같은 효과, 멱등).
fn open_up(root: &Path) {
    for d in [root.join(VAULT_DIR), root.join(VAULT_DIR).join(DATA_DIR)] {
        if d.exists() {
            let _ = fs::set_permissions(&d, fs::Permissions::from_mode(0o700));
        }
    }
}

/// 불변식: 원래의 모든 항목이 원래 자리 또는 보관 폴더에 내용 그대로 있다.
fn assert_nothing_lost(root: &Path, orig: &Snapshot, label: &str) {
    open_up(root);
    let data = root.join(VAULT_DIR).join(DATA_DIR);
    for (rel, entry) in orig {
        let candidates = [root.join(rel), data.join(rel)];
        let found = candidates.iter().any(|p| match entry {
            Entry::Dir => p.is_dir(),
            Entry::File(bytes) => fs::read(p).map(|b| &b == bytes).unwrap_or(false),
        });
        assert!(found, "[{label}] 유실: {}", rel.display());
    }
}

/// 프로그램을 다시 켰을 때의 복구 절차 → 최종적으로 잠금 해제까지.
fn recover_and_unlock(f: &Fixture, label: &str) {
    let e = engine(OsPlatform);
    let status = match e.resume(&f.root, Some(&f.exe)) {
        Ok(s) => s,
        Err(Error::Broken) => {
            e.emergency_restore(&f.root).unwrap_or_else(|e| panic!("[{label}] 비상 복원 실패: {e}"));
            Status::Unlocked
        }
        Err(err) => panic!("[{label}] 복구 실패: {err}"),
    };
    if status == Status::Locked {
        e.unlock(&f.root, PW, None).unwrap_or_else(|e| panic!("[{label}] 해제 실패: {e}"));
    }
    assert_eq!(e.status(&f.root).unwrap(), Status::Unlocked, "[{label}]");
}

fn count_ops(f: impl FnOnce(&Engine<Faulty>)) -> usize {
    let e = engine(Faulty::default());
    f(&e);
    e.platform.count.get()
}

// ---------- 기본 동작 ----------

#[test]
fn lock_unlock_roundtrip_many_trees() {
    for seed in 0..40 {
        let f = fixture(seed);
        let orig = snapshot(&f.root);
        let e = engine(OsPlatform);

        let report = e.lock(&f.root, &f.hash, Some(&f.exe)).unwrap();
        assert!(report.warnings.is_empty(), "{:?}", report.warnings);
        assert_eq!(e.status(&f.root).unwrap(), Status::Locked);

        let mut visible: Vec<_> = fs::read_dir(&f.root).unwrap().map(|x| x.unwrap().file_name()).collect();
        visible.sort();
        assert_eq!(visible, vec![OsString::from(VAULT_DIR), OsString::from(UNLOCK_EXE_NAME)]);
        let err = fs::read_dir(f.root.join(VAULT_DIR)).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::PermissionDenied, "보관 폴더 목록이 보이면 안 됨");

        assert!(matches!(e.unlock(&f.root, "wrong", None), Err(Error::WrongPassword)));
        assert_eq!(e.status(&f.root).unwrap(), Status::Locked);

        let report = e.unlock(&f.root, PW, None).unwrap();
        assert!(report.warnings.is_empty(), "{:?}", report.warnings);
        assert_eq!(snapshot(&f.root), orig, "seed {seed}");
    }
}

#[test]
fn lock_without_unlock_exe() {
    let f = fixture(7);
    let orig = snapshot(&f.root);
    let e = engine(OsPlatform);
    e.lock(&f.root, &f.hash, None).unwrap();
    let visible: Vec<_> = fs::read_dir(&f.root).unwrap().map(|x| x.unwrap().file_name()).collect();
    assert_eq!(visible, vec![OsString::from(VAULT_DIR)]);
    e.unlock(&f.root, PW, None).unwrap();
    assert_eq!(snapshot(&f.root), orig);
}

#[test]
fn empty_folder_roundtrip() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("빈");
    fs::create_dir(&root).unwrap();
    let e = engine(OsPlatform);
    e.lock(&root, &hash_password(PW), None).unwrap();
    e.unlock(&root, PW, None).unwrap();
    assert!(snapshot(&root).is_empty());
}

#[test]
fn double_lock_is_refused() {
    let f = fixture(3);
    let e = engine(OsPlatform);
    e.lock(&f.root, &f.hash, Some(&f.exe)).unwrap();
    assert!(matches!(e.lock(&f.root, &f.hash, Some(&f.exe)), Err(Error::AlreadyLocked)));
    e.unlock(&f.root, PW, None).unwrap();
    assert!(matches!(e.unlock(&f.root, PW, None), Err(Error::NotLocked)));
}

#[test]
fn file_created_while_locked_is_never_overwritten() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("r");
    fs::create_dir(&root).unwrap();
    fs::write(root.join("보고서.docx"), b"original").unwrap();
    fs::create_dir(root.join("사진")).unwrap();
    fs::write(root.join("사진/a.jpg"), b"jpg").unwrap();
    let e = engine(OsPlatform);
    e.lock(&root, &hash_password(PW), None).unwrap();

    // 잠긴 동안 같은 이름으로 새 항목을 만든다.
    fs::write(root.join("보고서.docx"), b"new while locked").unwrap();
    fs::create_dir(root.join("사진")).unwrap();

    let report = e.unlock(&root, PW, None).unwrap();
    assert_eq!(report.renamed.len(), 2);
    assert_eq!(fs::read(root.join("보고서.docx")).unwrap(), b"new while locked");
    assert_eq!(fs::read(root.join("보고서 (복원 1).docx")).unwrap(), b"original");
    assert_eq!(fs::read(root.join("사진 (복원 1)/a.jpg")).unwrap(), b"jpg");
    assert!(root.join("사진").is_dir());
}

#[test]
fn user_file_with_unlock_exe_name_survives() {
    let f = fixture(11);
    fs::write(f.root.join(UNLOCK_EXE_NAME), b"user's own file").unwrap();
    let orig = snapshot(&f.root);
    let e = engine(OsPlatform);
    e.lock(&f.root, &f.hash, Some(&f.exe)).unwrap();
    let report = e.unlock(&f.root, PW, None).unwrap();
    assert!(report.renamed.is_empty());
    assert_eq!(snapshot(&f.root), orig);
}

#[test]
fn tampered_unlock_exe_is_not_deleted() {
    let f = fixture(12);
    let e = engine(OsPlatform);
    e.lock(&f.root, &f.hash, Some(&f.exe)).unwrap();
    fs::write(f.root.join(UNLOCK_EXE_NAME), b"someone replaced it").unwrap();
    let report = e.unlock(&f.root, PW, None).unwrap();
    assert_eq!(report.warnings.len(), 1);
    assert_eq!(fs::read(f.root.join(UNLOCK_EXE_NAME)).unwrap(), b"someone replaced it");
}

#[test]
fn password_change_while_locked() {
    let f = fixture(5);
    let orig = snapshot(&f.root);
    let e = engine(OsPlatform);
    e.lock(&f.root, &f.hash, Some(&f.exe)).unwrap();
    e.set_password_hash(&f.root, &hash_password("새 비밀번호")).unwrap();
    assert!(matches!(e.unlock(&f.root, PW, None), Err(Error::WrongPassword)));
    e.unlock(&f.root, "새 비밀번호", None).unwrap();
    assert_eq!(snapshot(&f.root), orig);
}

// ---------- 실패·중단 ----------

#[test]
fn file_in_use_rolls_back_completely() {
    let f = fixture(21);
    let orig = snapshot(&f.root);
    let top: Vec<OsString> = {
        let mut v: Vec<_> = fs::read_dir(&f.root).unwrap().map(|x| x.unwrap().file_name()).collect();
        v.sort();
        v
    };
    // 가운데 항목이 사용 중이라 옮길 수 없는 상황.
    let busy = top[top.len() / 2].clone();
    let e = engine(Faulty { fail_move_of: Some(busy), ..Default::default() });
    let err = e.lock(&f.root, &f.hash, Some(&f.exe)).unwrap_err();
    assert!(matches!(err, Error::Io { .. }), "{err}");
    assert_eq!(snapshot(&f.root), orig, "실패 후 원래 상태 그대로여야 함");
}

#[test]
fn transient_failure_at_every_lock_step_rolls_back() {
    let f0 = fixture(31);
    let total = count_ops(|e| {
        e.lock(&f0.root, &f0.hash, Some(&f0.exe)).unwrap();
    });
    for at in 0..total {
        let f = fixture(31);
        let orig = snapshot(&f.root);
        let e = engine(Faulty { fail_once_at: Some(at), ..Default::default() });
        let result = e.lock(&f.root, &f.hash, Some(&f.exe));
        match result {
            // exe 복사 실패는 경고로만 처리되어 잠금은 성공한다.
            Ok(r) => {
                assert!(!r.warnings.is_empty(), "op {at}: 실패를 삼키면 안 됨");
                recover_and_unlock(&f, &format!("transient ok {at}"));
            }
            Err(_) => {
                assert_nothing_lost(&f.root, &orig, &format!("transient {at}"));
                recover_and_unlock(&f, &format!("transient {at}"));
            }
        }
        assert_eq!(snapshot(&f.root), orig, "op {at}");
    }
}

#[test]
fn crash_at_every_lock_step_is_recoverable() {
    for seed in [1, 2, 3, 4] {
        let f0 = fixture(seed);
        let total = count_ops(|e| {
            e.lock(&f0.root, &f0.hash, Some(&f0.exe)).unwrap();
        });
        assert!(total > 5);
        for at in 0..=total {
            let f = fixture(seed);
            let orig = snapshot(&f.root);
            let _ = engine(Faulty::crash(at)).lock(&f.root, &f.hash, Some(&f.exe));
            let label = format!("seed {seed} lock crash {at}/{total}");
            assert_nothing_lost(&f.root, &orig, &label);
            recover_and_unlock(&f, &label);
            assert_eq!(snapshot(&f.root), orig, "{label}");
        }
    }
}

#[test]
fn crash_at_every_unlock_step_is_recoverable() {
    for seed in [5, 6, 7] {
        let f0 = fixture(seed);
        engine(OsPlatform).lock(&f0.root, &f0.hash, Some(&f0.exe)).unwrap();
        let total = count_ops(|e| {
            e.unlock(&f0.root, PW, None).unwrap();
        });
        for at in 0..=total {
            let f = fixture(seed);
            let orig = snapshot(&f.root);
            engine(OsPlatform).lock(&f.root, &f.hash, Some(&f.exe)).unwrap();
            let _ = engine(Faulty::crash(at)).unlock(&f.root, PW, None);
            let label = format!("seed {seed} unlock crash {at}/{total}");
            assert_nothing_lost(&f.root, &orig, &label);
            recover_and_unlock(&f, &label);
            assert_eq!(snapshot(&f.root), orig, "{label}");
        }
    }
}

#[test]
fn crash_during_rollback_is_recoverable() {
    let seed = 41;
    let f0 = fixture(seed);
    let total = count_ops(|e| {
        e.lock(&f0.root, &f0.hash, Some(&f0.exe)).unwrap();
    });
    for fail in 0..total {
        for extra in 1..12 {
            let f = fixture(seed);
            let orig = snapshot(&f.root);
            let p = Faulty { fail_once_at: Some(fail), crash_at: Some(fail + extra), ..Default::default() };
            let _ = engine(p).lock(&f.root, &f.hash, Some(&f.exe));
            let label = format!("rollback fail {fail} crash +{extra}");
            assert_nothing_lost(&f.root, &orig, &label);
            recover_and_unlock(&f, &label);
            assert_eq!(snapshot(&f.root), orig, "{label}");
        }
    }
}

#[test]
fn repeated_crashes_during_recovery_are_recoverable() {
    // 복구 도중에 또 꺼지는 경우까지: 복구를 여러 번 중단시킨 뒤 마지막에 정상 복구.
    let seed = 51;
    let f0 = fixture(seed);
    let total = count_ops(|e| {
        e.lock(&f0.root, &f0.hash, Some(&f0.exe)).unwrap();
    });
    for at in 0..total {
        let f = fixture(seed);
        let orig = snapshot(&f.root);
        let _ = engine(Faulty::crash(at)).lock(&f.root, &f.hash, Some(&f.exe));
        for second in [1, 2, 3] {
            let _ = engine(Faulty::crash(second)).resume(&f.root, Some(&f.exe));
            assert_nothing_lost(&f.root, &orig, &format!("lock crash {at}, resume crash {second}"));
        }
        recover_and_unlock(&f, &format!("lock crash {at}"));
        assert_eq!(snapshot(&f.root), orig, "lock crash {at}");
    }
}

// ---------- 상태 파일 손상 ----------

#[test]
fn stale_vault_is_cleaned_and_lock_proceeds() {
    let f = fixture(61);
    let orig = snapshot(&f.root);
    fs::create_dir(f.root.join(VAULT_DIR)).unwrap();
    let e = engine(OsPlatform);
    assert_eq!(e.status(&f.root).unwrap(), Status::Stale);
    e.lock(&f.root, &f.hash, None).unwrap();
    e.unlock(&f.root, PW, None).unwrap();
    // 처음에 우리가 만든 빈 .folderlock도 정리된다.
    assert_eq!(snapshot(&f.root), orig);
}

#[test]
fn missing_meta_needs_fallback_or_emergency() {
    let f = fixture(62);
    let orig = snapshot(&f.root);
    let e = engine(OsPlatform);
    e.lock(&f.root, &f.hash, None).unwrap();
    open_up(&f.root);
    fs::remove_file(f.root.join(VAULT_DIR).join("meta.json")).unwrap();
    assert_eq!(e.status(&f.root).unwrap(), Status::Broken);
    assert!(matches!(e.unlock(&f.root, PW, None), Err(Error::NoPasswordInfo)));
    assert!(matches!(e.unlock(&f.root, "wrong", Some(&f.hash)), Err(Error::WrongPassword)));
    e.unlock(&f.root, PW, Some(&f.hash)).unwrap();
    assert_eq!(snapshot(&f.root), orig);
}

#[test]
fn emergency_restore_without_meta() {
    let f = fixture(63);
    let orig = snapshot(&f.root);
    let e = engine(OsPlatform);
    e.lock(&f.root, &f.hash, None).unwrap();
    open_up(&f.root);
    fs::write(f.root.join(VAULT_DIR).join("meta.json"), b"{ corrupted").unwrap();
    assert_eq!(e.status(&f.root).unwrap(), Status::Broken);
    e.emergency_restore(&f.root).unwrap();
    assert_eq!(snapshot(&f.root), orig);
}

#[test]
fn interrupted_state_is_reported() {
    let f = fixture(64);
    let _ = engine(Faulty::crash(4)).lock(&f.root, &f.hash, None);
    assert_eq!(engine(OsPlatform).status(&f.root).unwrap(), Status::Interrupted(LockState::Locking));
}

// ---------- 잠그면 안 되는 폴더 ----------

#[test]
fn guard_rejects_dangerous_targets() {
    let tmp = tempfile::tempdir().unwrap();
    let base = tmp.path().to_path_buf();
    let target = base.join("t");
    fs::create_dir(&target).unwrap();
    fs::write(target.join("f"), b"x").unwrap();
    let h = hash_password(PW);

    let deny = |rules: Rules, path: &Path| {
        let e = Engine::new(OsPlatform, rules);
        let err = e.lock(path, &h, None).unwrap_err();
        assert!(matches!(err, Error::Guard(_)), "{err}");
        assert!(path.symlink_metadata().is_err() || !path.join(VAULT_DIR).exists());
    };

    deny(Rules::default(), Path::new("/"));
    deny(Rules::default(), Path::new("relative/path"));
    deny(Rules::default(), &target.join("f"));
    deny(Rules::default(), &base.join("missing"));

    let link = base.join("link");
    std::os::unix::fs::symlink(&target, &link).unwrap();
    deny(Rules::default(), &link);

    deny(Rules { forbidden_exact: vec![target.clone()], ..Default::default() }, &target);
    deny(Rules { forbidden_tree: vec![base.clone()], ..Default::default() }, &target);
    deny(Rules { must_not_contain: vec![target.join("f")], ..Default::default() }, &target);

    // 금지 폴더의 하위 폴더는 exact 규칙에선 허용된다.
    let sub = target.join("sub");
    fs::create_dir(&sub).unwrap();
    let e = Engine::new(OsPlatform, Rules { forbidden_exact: vec![target.clone()], ..Default::default() });
    e.lock(&sub, &h, None).unwrap();
    e.unlock(&sub, PW, None).unwrap();
    assert_eq!(fs::read(target.join("f")).unwrap(), b"x");
}
