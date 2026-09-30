//! A private copy of the made-up vault in tests/fixtures/vault, with fixed
//! modified times, so screens and "3d ago" come out the same every run.

#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::sync::Once;
use std::time::{Duration, SystemTime};

use obsidian_tui::harness::H;

/// 2026-09-30 12:00 UTC
pub const NOW: u64 = 1_790_769_600;

pub fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

pub fn setup() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| unsafe {
        std::env::set_var("OBSIDIAN_TUI_NOW", NOW.to_string());
        // a fixed theme, so nothing follows the desktop's
        std::env::set_var("OBSIDIAN_TUI_THEME_STATE", fixtures().join("no-theme"));
        std::env::set_var("OBSIDIAN_TUI_THEME", "dark");
    });
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap().flatten() {
        let p = e.path();
        let t = to.join(e.file_name());
        if p.is_dir() {
            copy_dir(&p, &t);
        } else {
            std::fs::copy(&p, &t).unwrap();
        }
    }
}

fn all_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for e in std::fs::read_dir(dir).unwrap().flatten() {
        let p = e.path();
        if p.is_dir() {
            all_files(&p, out);
        } else {
            out.push(p);
        }
    }
}

/// A fresh copy of the vault in a temp folder called "Fern Vault". Notes get
/// modified times a day apart, in name order, the newest last.
pub fn vault(name: &str) -> (PathBuf, PathBuf) {
    setup();
    let base = std::env::temp_dir().join(format!("obsidian-tui-test-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let v = base.join("Fern Vault");
    copy_dir(&fixtures().join("vault"), &v);
    let mut files = vec![];
    all_files(&v, &mut files);
    files.sort();
    for (i, f) in files.iter().enumerate() {
        let t = SystemTime::UNIX_EPOCH + Duration::from_secs(NOW - 86_400 * (files.len() - i) as u64);
        let fh = std::fs::File::options().write(true).open(f).unwrap();
        fh.set_modified(t).unwrap();
    }
    (v, base.join("state.json"))
}

pub fn h(name: &str, w: u16, ht: u16) -> H {
    let (v, st) = vault(name);
    H::new(&v, &st, w, ht)
}

/// Every file in the vault with its bytes, to prove nothing was written.
pub fn snapshot(v: &Path) -> Vec<(PathBuf, Vec<u8>, SystemTime)> {
    let mut files = vec![];
    all_files(v, &mut files);
    files.sort();
    files
        .into_iter()
        .map(|f| {
            let b = std::fs::read(&f).unwrap();
            let m = std::fs::metadata(&f).unwrap().modified().unwrap();
            (f, b, m)
        })
        .collect()
}

pub fn golden(name: &str, screen: &str) {
    let p = fixtures().parent().unwrap().join("golden").join(format!("{name}.txt"));
    if std::env::var("UPDATE_GOLDEN").is_ok() || !p.exists() {
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, screen).unwrap();
        return;
    }
    let want = std::fs::read_to_string(&p).unwrap();
    if want != screen {
        let diff: Vec<String> = want
            .lines()
            .zip(screen.lines())
            .enumerate()
            .filter(|(_, (a, b))| a != b)
            .map(|(i, (a, b))| format!("line {i}\n  want: {a}\n  got:  {b}"))
            .collect();
        panic!("{name} differs from tests/golden/{name}.txt (UPDATE_GOLDEN=1 rewrites):\n{}", diff.join("\n"));
    }
}

pub fn sel(t: &H) -> String {
    t.app.sel.clone().unwrap_or_default()
}

pub fn note(t: &H) -> String {
    t.app.note.clone().unwrap_or_default()
}
