//! A small, modeless editor for one note, in the spirit of nano: type, move
//! with the arrows, select with shift or the mouse, ctrl+c/x/v, ctrl+z/y,
//! ctrl+f, ctrl+s. Long lines wrap softly.
//!
//! Saving is careful: the file's line endings, byte-order mark and final
//! newline are kept, the new text goes to a hidden temp file beside the note
//! and is renamed over it, and if the file changed on disk since it was
//! opened the caller is told rather than it being overwritten.

use std::path::{Path, PathBuf};

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::input::{Ed, LineEdit};
use crate::util::char_w;

const TAB_W: usize = 4;
const UNDO_MAX: usize = 500;

/// Cells a character takes in the editor: tabs are four wide.
pub fn ew(c: char) -> usize {
    if c == '\t' { TAB_W } else { char_w(c).max(1) }
}

#[derive(Clone)]
struct Snap {
    lines: Vec<Vec<char>>,
    row: usize,
    col: usize,
}

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    None,
    Type,
    Other,
}

/// One wrapped row on screen: a logical line and the columns it shows.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VRow {
    pub line: usize,
    pub start: usize,
    pub end: usize,
}

/// What a key asks the app to do.
#[derive(Debug, PartialEq)]
pub enum Out {
    Nothing,
    Save,
    Leave,
    Copy(String),
    Paste,
}

pub enum Saved {
    Ok,
    /// someone else changed (or moved) the file since it was opened
    ChangedOnDisk,
    Err(String),
}

/// The system clipboard where there is one, with a copy kept here too.
#[derive(Default)]
pub struct Clip {
    pub internal: String,
    sys: Option<arboard::Clipboard>,
    tried: bool,
    /// tests: never touch the real clipboard
    pub off: bool,
}

impl Clip {
    fn sys(&mut self) -> Option<&mut arboard::Clipboard> {
        if self.off {
            return None;
        }
        if !self.tried {
            self.tried = true;
            self.sys = arboard::Clipboard::new().ok();
        }
        self.sys.as_mut()
    }

    pub fn set(&mut self, s: &str) {
        self.internal = s.to_string();
        if let Some(c) = self.sys() {
            let _ = c.set_text(s.to_string());
        }
    }

    pub fn get(&mut self) -> String {
        let got = self.sys().and_then(|c| c.get_text().ok());
        match got {
            Some(s) => s,
            None => self.internal.clone(),
        }
    }
}

pub struct Editor {
    pub key: String,
    /// the real file (a symlink is followed, so saving keeps the link)
    pub path: PathBuf,
    pub lines: Vec<Vec<char>>,
    pub row: usize,
    pub col: usize,
    pub anchor: Option<(usize, usize)>,
    want_x: Option<usize>,
    pub scroll: usize,
    /// keep the cursor on screen at the next draw
    pub follow: bool,
    undo: Vec<Snap>,
    redo: Vec<Snap>,
    last: Kind,
    pub modified: bool,
    saved: String,
    /// the bytes on disk when opened or last saved
    orig: Vec<u8>,
    pub crlf: bool,
    bom: bool,
    pub indent: String,
    /// text width and height at the last draw
    pub width: usize,
    pub height: usize,
    pub find: Option<LineEdit>,
    pub find_miss: bool,
    /// the mouse button is down in the text
    pub selecting: bool,
}

fn chars(s: &str) -> Vec<Vec<char>> {
    s.split('\n').map(|l| l.chars().collect()).collect()
}

/// Tabs if the file indents with tabs, else its space width, else a tab
/// (Obsidian's own default).
fn guess_indent(text: &str) -> String {
    let mut spaces: Option<usize> = None;
    for l in text.lines() {
        if l.starts_with('\t') {
            return "\t".into();
        }
        let n = l.chars().take_while(|c| *c == ' ').count();
        if n >= 2 && l.chars().nth(n).is_some_and(|c| !c.is_whitespace()) {
            spaces = Some(spaces.map_or(n, |s| s.min(n)));
        }
    }
    match spaces {
        Some(n) => " ".repeat(n.clamp(2, 4)),
        None => "\t".into(),
    }
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

impl Editor {
    /// Open a note's file. Refuses anything that isn't UTF-8 text.
    pub fn open(key: &str, path: &Path) -> Result<Editor, String> {
        let real = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        let bytes = std::fs::read(&real).map_err(|e| format!("Couldn't read it: {e}"))?;
        let (bom, body) = match bytes.strip_prefix(b"\xEF\xBB\xBF") {
            Some(b) => (true, b),
            None => (false, &bytes[..]),
        };
        let text = std::str::from_utf8(body)
            .map_err(|_| "It isn't UTF-8 text, so it can't be edited here. Press E for your own editor.".to_string())?;
        let crlf_n = text.matches("\r\n").count();
        let lf_n = text.matches('\n').count() - crlf_n;
        let crlf = crlf_n > 0 && crlf_n >= lf_n;
        let text = text.replace("\r\n", "\n");
        Ok(Editor {
            key: key.to_string(),
            path: real,
            lines: chars(&text),
            row: 0,
            col: 0,
            anchor: None,
            want_x: None,
            scroll: 0,
            follow: true,
            undo: vec![],
            redo: vec![],
            last: Kind::None,
            modified: false,
            indent: guess_indent(&text),
            saved: text,
            orig: bytes,
            crlf,
            bom,
            width: 60,
            height: 20,
            find: None,
            find_miss: false,
            selecting: false,
        })
    }

    pub fn text(&self) -> String {
        self.lines.iter().map(|l| l.iter().collect::<String>()).collect::<Vec<_>>().join("\n")
    }

    fn refresh_modified(&mut self) {
        self.modified = self.text() != self.saved;
    }

    /// What the file's bytes would be now.
    pub fn bytes(&self) -> Vec<u8> {
        let mut out = if self.bom { b"\xEF\xBB\xBF".to_vec() } else { vec![] };
        let t = self.text();
        let t = if self.crlf { t.replace('\n', "\r\n") } else { t };
        out.extend_from_slice(t.as_bytes());
        out
    }

    /// Write it out: temp file beside it, then rename over. Unless `force`,
    /// stops if the file on disk isn't what was opened.
    pub fn save(&mut self, force: bool) -> Saved {
        if !force {
            match std::fs::read(&self.path) {
                Ok(now) if now == self.orig => {}
                _ => return Saved::ChangedOnDisk,
            }
        }
        let dir = self.path.parent().map(Path::to_path_buf).unwrap_or_else(|| PathBuf::from("."));
        let name = self.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "note".into());
        let tmp = dir.join(format!(".{name}.{}.obsidian-tui.tmp", std::process::id()));
        let bytes = self.bytes();
        let write = || -> std::io::Result<()> {
            use std::io::Write;
            let mut f = std::fs::File::create(&tmp)?;
            f.write_all(&bytes)?;
            f.sync_all()?;
            drop(f);
            if let Ok(m) = std::fs::metadata(&self.path) {
                let _ = std::fs::set_permissions(&tmp, m.permissions());
            }
            std::fs::rename(&tmp, &self.path)
        };
        match write() {
            Ok(()) => {
                self.orig = bytes;
                self.saved = self.text();
                self.modified = false;
                Saved::Ok
            }
            Err(e) => {
                let _ = std::fs::remove_file(&tmp);
                Saved::Err(e.to_string())
            }
        }
    }

    /// Throw away the edits and load what's on disk now.
    pub fn reload(&mut self) -> Result<(), String> {
        let fresh = Editor::open(&self.key, &self.path)?;
        let (row, col, scroll) = (self.row, self.col, self.scroll);
        *self = Editor { width: self.width, height: self.height, ..fresh };
        self.row = row.min(self.lines.len() - 1);
        self.col = col.min(self.lines[self.row].len());
        self.scroll = scroll;
        Ok(())
    }

    // ------------------------------------------------------------ layout
    /// Soft-wrapped rows for a text `w` cells wide, breaking after a space
    /// where one fits.
    pub fn layout(&self, w: usize) -> Vec<VRow> {
        let w = w.max(4);
        let mut out = vec![];
        for (li, line) in self.lines.iter().enumerate() {
            let mut start = 0;
            loop {
                let mut used = 0;
                let mut end = start;
                while end < line.len() && used + ew(line[end]) <= w {
                    used += ew(line[end]);
                    end += 1;
                }
                if end == start && end < line.len() {
                    end += 1;
                }
                if end < line.len()
                    && let Some(sp) = (start + 1..=end).rev().find(|&i| line[i - 1] == ' ')
                {
                    end = sp;
                }
                out.push(VRow { line: li, start, end });
                if end >= line.len() {
                    break;
                }
                start = end;
            }
        }
        out
    }

    fn x_of(&self, r: &VRow, col: usize) -> usize {
        self.lines[r.line][r.start..col.min(r.end).max(r.start)].iter().map(|c| ew(*c)).sum()
    }

    /// The row on screen (in `lay`) the position lands on, and its x.
    pub fn vis(&self, lay: &[VRow], row: usize, col: usize) -> (usize, usize) {
        for (i, r) in lay.iter().enumerate() {
            if r.line != row {
                continue;
            }
            let last = lay.get(i + 1).is_none_or(|n| n.line != row);
            if col >= r.start && (col < r.end || last) {
                return (i, self.x_of(r, col));
            }
        }
        (0, 0)
    }

    /// The column in screen row `vr` nearest x.
    pub fn col_at(&self, lay: &[VRow], vr: usize, x: usize) -> (usize, usize) {
        let Some(r) = lay.get(vr.min(lay.len().saturating_sub(1))) else { return (0, 0) };
        let line = &self.lines[r.line];
        let last = lay.get(vr + 1).is_none_or(|n| n.line != r.line);
        // the char under cell x; past the end, the end
        let mut at = 0;
        let mut c = r.start;
        while c < r.end {
            let cw = ew(line[c]);
            if x < at + cw {
                break;
            }
            at += cw;
            c += 1;
        }
        // the end of a wrapped row belongs to the next one
        if c == r.end && !last && r.end > r.start {
            c = r.end - 1;
        }
        (r.line, c.min(line.len()))
    }

    /// Keep the cursor inside the `h` rows on screen.
    pub fn keep_visible(&mut self, lay: &[VRow], h: usize) {
        let (vr, _) = self.vis(lay, self.row, self.col);
        let h = h.max(1);
        if vr < self.scroll {
            self.scroll = vr;
        } else if vr >= self.scroll + h {
            self.scroll = vr + 1 - h;
        }
    }

    // ------------------------------------------------------------ selection
    pub fn sel(&self) -> Option<((usize, usize), (usize, usize))> {
        let a = self.anchor?;
        let b = (self.row, self.col);
        if a == b {
            return None;
        }
        Some(if a < b { (a, b) } else { (b, a) })
    }

    pub fn selected(&self) -> Option<String> {
        let ((r0, c0), (r1, c1)) = self.sel()?;
        if r0 == r1 {
            return Some(self.lines[r0][c0..c1].iter().collect());
        }
        let mut s: String = self.lines[r0][c0..].iter().collect();
        for l in &self.lines[r0 + 1..r1] {
            s.push('\n');
            s.extend(l.iter());
        }
        s.push('\n');
        s.extend(self.lines[r1][..c1].iter());
        Some(s)
    }

    fn delete_sel(&mut self) -> bool {
        let Some(((r0, c0), (r1, c1))) = self.sel() else { return false };
        let tail: Vec<char> = self.lines[r1][c1..].to_vec();
        self.lines[r0].truncate(c0);
        self.lines[r0].extend(tail);
        self.lines.drain(r0 + 1..=r1);
        self.row = r0;
        self.col = c0;
        self.anchor = None;
        true
    }

    pub fn select_all(&mut self) {
        self.anchor = Some((0, 0));
        self.row = self.lines.len() - 1;
        self.col = self.lines[self.row].len();
        self.follow = true;
    }

    // ------------------------------------------------------------ editing
    fn checkpoint(&mut self, kind: Kind) {
        if kind == Kind::Type && self.last == Kind::Type {
            return;
        }
        self.undo.push(Snap { lines: self.lines.clone(), row: self.row, col: self.col });
        if self.undo.len() > UNDO_MAX {
            self.undo.remove(0);
        }
        self.redo.clear();
        self.last = kind;
    }

    fn changed(&mut self) {
        self.want_x = None;
        self.follow = true;
        self.refresh_modified();
    }

    /// Put text in at the cursor, over any selection.
    pub fn insert(&mut self, s: &str) {
        let kind = if s.chars().count() == 1 && !s.contains('\n') && self.sel().is_none() { Kind::Type } else { Kind::Other };
        self.checkpoint(kind);
        self.delete_sel();
        let s = s.replace("\r\n", "\n").replace('\r', "\n");
        for c in s.chars() {
            if c == '\n' {
                let rest = self.lines[self.row].split_off(self.col);
                self.lines.insert(self.row + 1, rest);
                self.row += 1;
                self.col = 0;
            } else {
                self.lines[self.row].insert(self.col, c);
                self.col += 1;
            }
        }
        if kind != Kind::Type || s == " " {
            // a word per undo step
            self.last = Kind::Other;
        }
        self.changed();
    }

    fn backspace(&mut self, word: bool) {
        self.checkpoint(Kind::Other);
        if !self.delete_sel() {
            if self.col > 0 {
                let to = if word { self.word_left() } else { self.col - 1 };
                self.lines[self.row].drain(to..self.col);
                self.col = to;
            } else if self.row > 0 {
                let line = self.lines.remove(self.row);
                self.row -= 1;
                self.col = self.lines[self.row].len();
                self.lines[self.row].extend(line);
            }
        }
        self.changed();
    }

    fn delete(&mut self, word: bool) {
        self.checkpoint(Kind::Other);
        if !self.delete_sel() {
            if self.col < self.lines[self.row].len() {
                let to = if word { self.word_right() } else { self.col + 1 };
                self.lines[self.row].drain(self.col..to);
            } else if self.row + 1 < self.lines.len() {
                let next = self.lines.remove(self.row + 1);
                self.lines[self.row].extend(next);
            }
        }
        self.changed();
    }

    fn outdent(&mut self) {
        let line = &self.lines[self.row];
        let n = if line.first() == Some(&'\t') {
            1
        } else {
            line.iter().take(self.indent.len().max(2)).take_while(|c| **c == ' ').count()
        };
        if n == 0 {
            return;
        }
        self.checkpoint(Kind::Other);
        self.lines[self.row].drain(..n);
        self.col = self.col.saturating_sub(n);
        self.changed();
    }

    pub fn undo(&mut self) {
        if let Some(s) = self.undo.pop() {
            self.redo.push(Snap { lines: self.lines.clone(), row: self.row, col: self.col });
            self.restore(s);
        }
    }

    pub fn redo(&mut self) {
        if let Some(s) = self.redo.pop() {
            self.undo.push(Snap { lines: self.lines.clone(), row: self.row, col: self.col });
            self.restore(s);
        }
    }

    fn restore(&mut self, s: Snap) {
        self.lines = s.lines;
        self.row = s.row.min(self.lines.len() - 1);
        self.col = s.col.min(self.lines[self.row].len());
        self.anchor = None;
        self.last = Kind::None;
        self.changed();
    }

    // ------------------------------------------------------------ moving
    fn word_left(&self) -> usize {
        let l = &self.lines[self.row];
        let mut i = self.col;
        while i > 0 && !is_word(l[i - 1]) {
            i -= 1;
        }
        while i > 0 && is_word(l[i - 1]) {
            i -= 1;
        }
        i
    }

    fn word_right(&self) -> usize {
        let l = &self.lines[self.row];
        let mut i = self.col;
        while i < l.len() && !is_word(l[i]) {
            i += 1;
        }
        while i < l.len() && is_word(l[i]) {
            i += 1;
        }
        i
    }

    /// Before a move: shift starts or keeps a selection, else it goes.
    fn mark(&mut self, shift: bool) {
        if shift {
            if self.anchor.is_none() {
                self.anchor = Some((self.row, self.col));
            }
        } else {
            self.anchor = None;
        }
        self.last = Kind::None;
        self.follow = true;
    }

    fn vertical(&mut self, d: i64) {
        let lay = self.layout(self.width);
        let (vr, x) = self.vis(&lay, self.row, self.col);
        let x = *self.want_x.get_or_insert(x);
        let target = vr as i64 + d;
        if target < 0 {
            self.row = 0;
            self.col = 0;
            return;
        }
        if target as usize >= lay.len() {
            self.row = self.lines.len() - 1;
            self.col = self.lines[self.row].len();
            return;
        }
        let (r, c) = self.col_at(&lay, target as usize, x);
        self.row = r;
        self.col = c;
    }

    /// Place the cursor at a spot on screen (row within the text area, x).
    pub fn click(&mut self, vr: usize, x: usize, extend: bool) {
        let lay = self.layout(self.width);
        let (r, c) = self.col_at(&lay, self.scroll + vr, x);
        if extend {
            if self.anchor.is_none() {
                self.anchor = Some((self.row, self.col));
            }
        } else {
            self.anchor = Some((r, c));
        }
        self.row = r;
        self.col = c;
        self.want_x = None;
        self.last = Kind::None;
    }

    // ------------------------------------------------------------ find
    /// Find the next (or previous) match of the find box, from the cursor,
    /// ignoring case, and select it.
    pub fn find_next(&mut self, back: bool) -> bool {
        let Some(q) = self.find.as_ref().map(|f| f.value()) else { return false };
        if q.is_empty() {
            return false;
        }
        let q: Vec<char> = q.to_lowercase().chars().collect();
        let low: Vec<Vec<char>> = self.lines.iter().map(|l| l.iter().map(|c| c.to_lowercase().next().unwrap_or(*c)).collect()).collect();
        let hits: Vec<(usize, usize)> = low
            .iter()
            .enumerate()
            .flat_map(|(r, l)| {
                let q = &q;
                (0..l.len()).filter(move |&c| l[c..].starts_with(q)).map(move |c| (r, c))
            })
            .collect();
        self.find_miss = hits.is_empty();
        if hits.is_empty() {
            return false;
        }
        let here = self.sel().map(|(a, _)| a).unwrap_or((self.row, self.col));
        let pick = if back {
            hits.iter().rev().find(|h| **h < here).or(hits.last())
        } else {
            let from = if self.sel().is_some() { (here.0, here.1 + 1) } else { here };
            hits.iter().find(|h| **h >= from).or(hits.first())
        };
        let &(r, c) = pick.unwrap();
        self.anchor = Some((r, c));
        self.row = r;
        self.col = c + q.len();
        self.follow = true;
        self.want_x = None;
        true
    }

    // ------------------------------------------------------------ keys
    pub fn key(&mut self, k: &KeyEvent) -> Out {
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        let alt = k.modifiers.contains(KeyModifiers::ALT);
        let shift = k.modifiers.contains(KeyModifiers::SHIFT);
        if let Some(f) = &mut self.find {
            match k.code {
                KeyCode::Esc => {
                    self.find = None;
                    self.find_miss = false;
                }
                KeyCode::Enter | KeyCode::Down => {
                    self.find_next(shift);
                }
                KeyCode::Up => {
                    self.find_next(true);
                }
                KeyCode::Char('f') if ctrl => {
                    self.find_next(false);
                }
                KeyCode::Char('s') if ctrl => return Out::Save,
                _ => {
                    if let Ed::Changed = f.key(k) {
                        // find as you type, from where the search started
                        if let Some(((r, c), _)) = self.sel() {
                            self.row = r;
                            self.col = c;
                            self.anchor = None;
                        }
                        self.find_next(false);
                    }
                }
            }
            return Out::Nothing;
        }
        match k.code {
            KeyCode::Esc => return Out::Leave,
            KeyCode::Char('s') if ctrl => return Out::Save,
            KeyCode::Char('q') if ctrl => return Out::Leave,
            KeyCode::Char('z') if ctrl && shift => self.redo(),
            KeyCode::Char('Z') if ctrl => self.redo(),
            KeyCode::Char('z') if ctrl => self.undo(),
            KeyCode::Char('y') if ctrl => self.redo(),
            KeyCode::Char('a') if ctrl => self.select_all(),
            KeyCode::Char('f') if ctrl => {
                let seed = self.selected().filter(|s| !s.contains('\n')).unwrap_or_default();
                self.find = Some(LineEdit::new(&seed));
                self.find_miss = false;
            }
            KeyCode::Char('c') if ctrl => {
                let s = self.selected().unwrap_or_else(|| self.lines[self.row].iter().collect::<String>() + "\n");
                return Out::Copy(s);
            }
            KeyCode::Char('x') if ctrl => {
                let s = match self.selected() {
                    Some(s) => {
                        self.checkpoint(Kind::Other);
                        self.delete_sel();
                        s
                    }
                    None => {
                        // no selection: cut the whole line
                        self.checkpoint(Kind::Other);
                        let s = self.lines[self.row].iter().collect::<String>() + "\n";
                        if self.lines.len() > 1 {
                            self.lines.remove(self.row);
                            self.row = self.row.min(self.lines.len() - 1);
                        } else {
                            self.lines[0].clear();
                        }
                        self.col = 0;
                        s
                    }
                };
                self.changed();
                return Out::Copy(s);
            }
            KeyCode::Char('v') if ctrl => return Out::Paste,
            KeyCode::Char('h') if ctrl => self.backspace(false),
            KeyCode::Char('w') if ctrl => self.backspace(true),
            KeyCode::Char(c) if !ctrl && !alt => self.insert(&c.to_string()),
            KeyCode::Enter => self.insert("\n"),
            KeyCode::Tab => {
                let ind = self.indent.clone();
                self.insert(&ind);
            }
            KeyCode::BackTab => self.outdent(),
            KeyCode::Backspace => self.backspace(ctrl || alt),
            KeyCode::Delete => self.delete(ctrl || alt),
            KeyCode::Left => {
                let had = self.sel();
                self.mark(shift);
                match (had, shift) {
                    (Some((a, _)), false) => (self.row, self.col) = a,
                    _ if ctrl => {
                        if self.col == 0 && self.row > 0 {
                            self.row -= 1;
                            self.col = self.lines[self.row].len();
                        } else {
                            self.col = self.word_left();
                        }
                    }
                    _ if self.col > 0 => self.col -= 1,
                    _ if self.row > 0 => {
                        self.row -= 1;
                        self.col = self.lines[self.row].len();
                    }
                    _ => {}
                }
                self.want_x = None;
            }
            KeyCode::Right => {
                let had = self.sel();
                self.mark(shift);
                match (had, shift) {
                    (Some((_, b)), false) => (self.row, self.col) = b,
                    _ if ctrl => {
                        if self.col == self.lines[self.row].len() && self.row + 1 < self.lines.len() {
                            self.row += 1;
                            self.col = 0;
                        } else {
                            self.col = self.word_right();
                        }
                    }
                    _ if self.col < self.lines[self.row].len() => self.col += 1,
                    _ if self.row + 1 < self.lines.len() => {
                        self.row += 1;
                        self.col = 0;
                    }
                    _ => {}
                }
                self.want_x = None;
            }
            KeyCode::Up => {
                self.mark(shift);
                self.vertical(-1);
            }
            KeyCode::Down => {
                self.mark(shift);
                self.vertical(1);
            }
            KeyCode::PageUp => {
                self.mark(shift);
                let h = self.height.saturating_sub(1).max(1) as i64;
                self.vertical(-h);
                self.scroll = self.scroll.saturating_sub(h as usize);
            }
            KeyCode::PageDown => {
                self.mark(shift);
                let h = self.height.saturating_sub(1).max(1) as i64;
                self.vertical(h);
                self.scroll += h as usize;
            }
            KeyCode::Home => {
                self.mark(shift);
                if ctrl {
                    self.row = 0;
                }
                // first press: after the indent; again: the very start
                let first = self.lines[self.row].iter().take_while(|c| c.is_whitespace()).count();
                self.col = if self.col == first || ctrl { 0 } else { first };
                self.want_x = None;
            }
            KeyCode::End => {
                self.mark(shift);
                if ctrl {
                    self.row = self.lines.len() - 1;
                }
                self.col = self.lines[self.row].len();
                self.want_x = None;
            }
            _ => {}
        }
        Out::Nothing
    }

    /// Scroll the view without moving the cursor (the wheel).
    pub fn wheel(&mut self, d: i64) {
        let rows = self.layout(self.width).len();
        self.scroll = ((self.scroll as i64 + d).max(0) as usize).min(rows.saturating_sub(1));
        self.follow = false;
    }
}

// ------------------------------------------------------------ colour
/// How each char of the text should look: plain, a heading, a link, a tag,
/// code, a quote or list marker, or frontmatter.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Tint {
    Plain,
    Heading,
    Link,
    Tag,
    Code,
    Marker,
    Front,
}

pub fn tints(lines: &[Vec<char>]) -> Vec<Vec<Tint>> {
    let mut out = Vec::with_capacity(lines.len());
    let mut fence = false;
    let mut front = lines.first().is_some_and(|l| l.iter().collect::<String>().trim_end() == "---");
    for (i, l) in lines.iter().enumerate() {
        let s: String = l.iter().collect();
        let t = s.trim_start();
        if front {
            out.push(vec![Tint::Front; l.len()]);
            if i > 0 && (s.trim_end() == "---" || s.trim_end() == "...") {
                front = false;
            }
            continue;
        }
        if t.starts_with("```") || t.starts_with("~~~") {
            fence = !fence;
            out.push(vec![Tint::Code; l.len()]);
            continue;
        }
        if fence {
            out.push(vec![Tint::Code; l.len()]);
            continue;
        }
        let hashes = t.chars().take_while(|c| *c == '#').count();
        if (1..=6).contains(&hashes) && t.chars().nth(hashes).is_none_or(|c| c == ' ') {
            out.push(vec![Tint::Heading; l.len()]);
            continue;
        }
        let mut v = vec![Tint::Plain; l.len()];
        // quote and list markers
        let lead = l.iter().take_while(|c| c.is_whitespace()).count();
        let mut j = lead;
        while j < l.len() && l[j] == '>' {
            v[j] = Tint::Marker;
            j += 1;
            while j < l.len() && l[j] == ' ' {
                j += 1;
            }
        }
        if j < l.len() && matches!(l[j], '-' | '*' | '+') && l.get(j + 1) == Some(&' ') {
            v[j] = Tint::Marker;
            if l.get(j + 2) == Some(&'[') && l.get(j + 4) == Some(&']') {
                for x in v.iter_mut().skip(j + 2).take(3) {
                    *x = Tint::Marker;
                }
            }
        }
        // inline: `code`, [[links]], [text](url), #tags
        let mut c = 0;
        while c < l.len() {
            if l[c] == '`' {
                let end = (c + 1..l.len()).find(|&e| l[e] == '`').unwrap_or(l.len() - 1);
                for x in v.iter_mut().take(end + 1).skip(c) {
                    *x = Tint::Code;
                }
                c = end + 1;
                continue;
            }
            if l[c] == '['
                && l.get(c + 1) == Some(&'[')
                && let Some(e) = (c + 2..l.len().saturating_sub(1)).find(|&e| l[e] == ']' && l[e + 1] == ']')
            {
                let from = if c > 0 && l[c - 1] == '!' { c - 1 } else { c };
                for x in v.iter_mut().take(e + 2).skip(from) {
                    *x = Tint::Link;
                }
                c = e + 2;
                continue;
            }
            if l[c] == '['
                && let Some(close) = (c + 1..l.len()).find(|&e| l[e] == ']')
                && l.get(close + 1) == Some(&'(')
                && let Some(e) = (close + 2..l.len()).find(|&e| l[e] == ')')
            {
                for x in v.iter_mut().take(e + 1).skip(c) {
                    *x = Tint::Link;
                }
                c = e + 1;
                continue;
            }
            if l[c] == '#'
                && (c == 0 || l[c - 1].is_whitespace())
                && l.get(c + 1).is_some_and(|n| n.is_alphabetic() || *n == '_')
            {
                let mut e = c + 1;
                while e < l.len() && (l[e].is_alphanumeric() || matches!(l[e], '_' | '-' | '/')) {
                    e += 1;
                }
                for x in v.iter_mut().take(e).skip(c) {
                    *x = Tint::Tag;
                }
                c = e;
                continue;
            }
            c += 1;
        }
        out.push(v);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ed(text: &str) -> Editor {
        Editor {
            key: "n.md".into(),
            path: PathBuf::from("n.md"),
            lines: chars(text),
            row: 0,
            col: 0,
            anchor: None,
            want_x: None,
            scroll: 0,
            follow: true,
            undo: vec![],
            redo: vec![],
            last: Kind::None,
            modified: false,
            saved: text.into(),
            orig: text.as_bytes().to_vec(),
            crlf: false,
            bom: false,
            indent: guess_indent(text),
            width: 10,
            height: 5,
            find: None,
            find_miss: false,
            selecting: false,
        }
    }

    #[test]
    fn wraps_after_spaces() {
        let e = ed("the quick brown fox");
        let lay = e.layout(10);
        let rows: Vec<String> = lay.iter().map(|r| e.lines[r.line][r.start..r.end].iter().collect()).collect();
        assert_eq!(rows, vec!["the quick ", "brown fox"]);
        // the end of a wrapped row is the start of the next
        assert_eq!(e.vis(&lay, 0, 10), (1, 0));
        assert_eq!(e.vis(&lay, 0, 19), (1, 9));
    }

    #[test]
    fn indent_matches_the_file() {
        assert_eq!(guess_indent("- a\n\t- b"), "\t");
        assert_eq!(guess_indent("- a\n  - b\n    - c"), "  ");
        assert_eq!(guess_indent("plain"), "\t");
    }

    #[test]
    fn typing_is_one_undo_step_per_word() {
        let mut e = ed("");
        e.insert("a");
        e.insert("b");
        e.insert(" ");
        e.insert("c");
        assert_eq!(e.text(), "ab c");
        e.undo();
        assert_eq!(e.text(), "ab ");
        e.undo();
        assert_eq!(e.text(), "");
        e.redo();
        e.redo();
        assert_eq!(e.text(), "ab c");
    }

    #[test]
    fn selections_delete_across_lines() {
        let mut e = ed("one\ntwo\nthree");
        e.anchor = Some((0, 1));
        e.row = 2;
        e.col = 2;
        assert_eq!(e.selected().unwrap(), "ne\ntwo\nth");
        e.insert("X");
        assert_eq!(e.text(), "oXree");
    }

    #[test]
    fn tints_markdown() {
        let t = tints(&chars("# Head\nsee [[Note]] and #tag and `x`"));
        assert!(t[0].iter().all(|x| *x == Tint::Heading));
        assert_eq!(t[1][4], Tint::Link);
        assert_eq!(t[1][17], Tint::Tag);
        assert_eq!(t[1][27], Tint::Code);
    }
}
