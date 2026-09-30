//! Small helpers: cell widths, fitting text, "3d ago", opening things with
//! whatever the platform uses, the clipboard over OSC 52.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use unicode_width::UnicodeWidthStr;

/// The home folder on any platform (HOME, or USERPROFILE on Windows).
pub fn home() -> PathBuf {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

pub fn cell_len(s: &str) -> usize {
    UnicodeWidthStr::width(s)
}

pub fn char_w(c: char) -> usize {
    unicode_width::UnicodeWidthChar::width(c).unwrap_or(0)
}

/// Truncate to w cells with an ellipsis, pad to w.
pub fn fit(s: &str, w: usize) -> String {
    if w == 0 {
        return String::new();
    }
    let mut s = s.to_string();
    if cell_len(&s) > w {
        while cell_len(&s) > w - 1 {
            s.pop();
        }
        s = s.trim_end().to_string() + "…";
    }
    let pad = w.saturating_sub(cell_len(&s));
    s + &" ".repeat(pad)
}

/// Cut to w cells with an ellipsis, no padding.
pub fn crop(s: &str, w: usize) -> String {
    if cell_len(s) <= w {
        return s.to_string();
    }
    fit(s, w).trim_end().to_string()
}

pub fn rjust(s: &str, w: usize) -> String {
    let n = cell_len(s);
    if n >= w { s.to_string() } else { " ".repeat(w - n) + s }
}

/// Seconds since the epoch, for tests that want a fixed "now".
pub fn now() -> SystemTime {
    if let Ok(v) = std::env::var("OBSIDIAN_TUI_NOW")
        && let Ok(s) = v.parse::<u64>()
    {
        return SystemTime::UNIX_EPOCH + Duration::from_secs(s);
    }
    SystemTime::now()
}

/// "now", "5m", "3h", "2d", "6w", "1y".
pub fn ago(t: SystemTime) -> String {
    let s = now().duration_since(t).map(|d| d.as_secs()).unwrap_or(0);
    let (n, u) = match s {
        0..60 => return "now".into(),
        60..3600 => (s / 60, "m"),
        3600..86400 => (s / 3600, "h"),
        86400..1_209_600 => (s / 86400, "d"),
        1_209_600..31_536_000 => (s / 604_800, "w"),
        _ => (s / 31_536_000, "y"),
    };
    format!("{n}{u}")
}

/// A vault-relative path with forward slashes, whatever the platform uses.
pub fn rel_key(root: &Path, p: &Path) -> String {
    let r = p.strip_prefix(root).unwrap_or(p);
    r.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect::<Vec<_>>().join("/")
}

/// A forward-slash key back to a real path under the vault.
pub fn key_path(root: &Path, key: &str) -> PathBuf {
    let mut p = root.to_path_buf();
    for part in key.split('/').filter(|s| !s.is_empty()) {
        p.push(part);
    }
    p
}

/// Open a file or URL with the desktop's default handler. Never blocks.
pub fn open_external(target: &str) -> bool {
    use std::process::{Command, Stdio};
    let mut cmd = if cfg!(target_os = "windows") {
        // not `cmd /C start`: cmd would split an obsidian:// URL at its &
        let mut c = Command::new("rundll32");
        c.args(["url.dll,FileProtocolHandler", target]);
        c
    } else if cfg!(target_os = "macos") {
        let mut c = Command::new("open");
        c.arg(target);
        c
    } else {
        let mut c = Command::new("xdg-open");
        c.arg(target);
        c
    };
    cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn().is_ok()
}

/// The editor to hand a note to: $VISUAL, $EDITOR, then a sensible default.
pub fn editor() -> Vec<String> {
    for var in ["VISUAL", "EDITOR"] {
        if let Ok(v) = std::env::var(var)
            && !v.trim().is_empty()
        {
            return v.split_whitespace().map(str::to_string).collect();
        }
    }
    if cfg!(target_os = "windows") {
        return vec!["notepad".into()];
    }
    for e in ["nano", "vim", "vi"] {
        if which(e) {
            return vec![e.into()];
        }
    }
    vec!["vi".into()]
}

fn which(bin: &str) -> bool {
    std::env::var_os("PATH").is_some_and(|paths| std::env::split_paths(&paths).any(|d| d.join(bin).is_file()))
}

/// Put text on the clipboard with OSC 52, which Windows Terminal, iTerm2,
/// kitty, foot, ghostty, alacritty and tmux (with set-clipboard) all honour.
pub fn osc52(text: &str) -> String {
    use base64::Engine;
    format!("\x1b]52;c;{}\x07", base64::engine::general_purpose::STANDARD.encode(text))
}

/// Percent-encode for an obsidian:// URL.
pub fn url_enc(s: &str) -> String {
    percent_encoding::utf8_percent_encode(s, percent_encoding::NON_ALPHANUMERIC).to_string()
}

pub fn url_dec(s: &str) -> String {
    percent_encoding::percent_decode_str(s).decode_utf8_lossy().into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fit_truncates_with_ellipsis() {
        assert_eq!(fit("hello world", 6), "hello…");
        assert_eq!(fit("hi", 4), "hi  ");
        assert_eq!(crop("hello world", 6), "hello…");
        assert_eq!(crop("hi", 6), "hi");
    }

    #[test]
    fn keys_use_forward_slashes() {
        let root = Path::new("vault");
        let p = root.join("a").join("b.md");
        assert_eq!(rel_key(root, &p), "a/b.md");
        assert_eq!(key_path(root, "a/b.md"), p);
    }
}
