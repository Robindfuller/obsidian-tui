//! The vault: every Markdown note under the folder, read into memory, with
//! its frontmatter, tags and links worked out, plus the indexes that resolve
//! a link the way Obsidian does and find a note's backlinks. Read-only: this
//! never writes a file.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use std::time::SystemTime;

use regex::Regex;

use crate::util::{key_path, rel_key, url_dec};

#[derive(Clone, Debug)]
pub struct RawLink {
    /// the note or file part, as written ("Folder/Note", "Note.md", "img.png")
    pub target: String,
    pub heading: Option<String>,
    pub embed: bool,
    /// 0-based source line, for backlink context
    pub line: usize,
}

#[derive(Clone, Debug)]
pub struct Note {
    /// vault-relative path with forward slashes: "Folder/Note.md"
    pub key: String,
    /// the file name without .md, which is what Obsidian calls the note
    pub name: String,
    /// the folder key, "" at the root
    pub folder: String,
    pub text: String,
    pub lower: String,
    pub mtime: SystemTime,
    size: u64,
    /// frontmatter as (key, values)
    pub front: Vec<(String, Vec<String>)>,
    /// byte offset where the body starts, after any frontmatter
    pub body_start: usize,
    pub aliases: Vec<String>,
    pub tags: Vec<String>,
    pub links: Vec<RawLink>,
}

impl Note {
    pub fn body(&self) -> &str {
        &self.text[self.body_start..]
    }

    /// The line a 0-based body line number points at.
    pub fn body_line(&self, n: usize) -> &str {
        self.body().lines().nth(n).unwrap_or("")
    }

    pub fn has_tag(&self, tag: &str) -> bool {
        let t = tag.to_lowercase();
        self.tags.iter().any(|x| {
            let x = x.to_lowercase();
            x == t || x.starts_with(&format!("{t}/"))
        })
    }
}

#[derive(Clone, Debug)]
pub struct SearchHit {
    pub key: String,
    pub snippet: String,
    pub score: usize,
}

#[derive(Default)]
pub struct Vault {
    pub root: PathBuf,
    pub name: String,
    pub notes: Vec<Note>,
    pub by_key: HashMap<String, usize>,
    by_path: HashMap<String, usize>,
    by_name: HashMap<String, Vec<usize>>,
    by_alias: HashMap<String, Vec<usize>>,
    /// attachments: every non-Markdown file, by key
    pub files: Vec<String>,
    files_by_name: HashMap<String, Vec<String>>,
    files_by_path: HashMap<String, String>,
    /// every folder that holds a note somewhere below it, sorted
    pub folders: Vec<String>,
    /// notes under each folder, counting subfolders
    pub folder_count: HashMap<String, usize>,
    /// target key -> (source key, source body line)
    pub backlinks: HashMap<String, Vec<(String, usize)>>,
    /// lowercased tag -> (tag as first written, note keys)
    pub tags: BTreeMap<String, (String, Vec<String>)>,
    /// bumps on every rescan that changed something
    pub generation: u64,
}

static WIKI: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(!?)\[\[([^\[\]\n]+?)\]\]").unwrap());
static MDLINK: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(!?)\[[^\]\n]*\]\(([^)\n]+)\)").unwrap());
pub static TAG: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?:^|[\s(,;])#([\p{L}\p{N}_/\-]*[\p{L}_/\-][\p{L}\p{N}_/\-]*)").unwrap());

/// Hidden folders (.obsidian, .trash, .git) and files are not part of the vault.
fn hidden(name: &str) -> bool {
    name.starts_with('.')
}

pub fn is_note(p: &Path) -> bool {
    p.extension().is_some_and(|e| e.eq_ignore_ascii_case("md"))
}

/// Does a link destination name a scheme (https:, mailto:, obsidian:)?
pub fn is_url(s: &str) -> bool {
    let s = s.trim();
    match s.find(':') {
        // a drive letter like C:\ is not a scheme
        Some(i) if i > 1 => s[..i].chars().all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '-' || c == '.'),
        _ => false,
    }
}

/// Code blocks and code spans, blanked with spaces so byte offsets still line
/// up: links and tags inside code don't count.
pub fn mask_code(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut fence: Option<String> = None;
    for line in s.split_inclusive('\n') {
        let t = line.trim_start();
        let is_fence = t.starts_with("```") || t.starts_with("~~~");
        let blank = |l: &str| l.chars().map(|c| if c == '\n' { "\n".to_string() } else { " ".repeat(c.len_utf8()) }).collect::<String>();
        if let Some(f) = &fence {
            if is_fence && t.starts_with(f.as_str()) {
                fence = None;
            }
            out.push_str(&blank(line));
            continue;
        }
        if is_fence {
            fence = Some(t[..3].to_string());
            out.push_str(&blank(line));
            continue;
        }
        // code spans: `...`
        let mut in_code = false;
        for c in line.chars() {
            if c == '`' {
                in_code = !in_code;
                out.push(' ');
            } else if in_code && c != '\n' {
                out.push_str(&" ".repeat(c.len_utf8()));
            } else {
                out.push(c);
            }
        }
    }
    out
}

fn unquote(s: &str) -> String {
    let s = s.trim();
    if s.len() >= 2 && ((s.starts_with('"') && s.ends_with('"')) || (s.starts_with('\'') && s.ends_with('\''))) {
        s[1..s.len() - 1].to_string()
    } else {
        s.to_string()
    }
}

/// Frontmatter between --- lines at the top: the simple YAML Obsidian writes
/// (key: value, key: [a, b], and "- item" lists). Returns the pairs and where
/// the body starts.
pub fn frontmatter(text: &str) -> (Vec<(String, Vec<String>)>, usize) {
    let first = text.lines().next().unwrap_or("");
    if first.trim_end() != "---" {
        return (vec![], 0);
    }
    let mut pos = text.find('\n').map(|i| i + 1).unwrap_or(text.len());
    let start = pos;
    let mut end = None;
    for line in text[start..].split_inclusive('\n') {
        let l = line.trim_end();
        if l == "---" || l == "..." {
            end = Some((pos, pos + line.len()));
            break;
        }
        pos += line.len();
    }
    let Some((yaml_end, body)) = end else { return (vec![], 0) };
    let mut out: Vec<(String, Vec<String>)> = vec![];
    for line in text[start..yaml_end].lines() {
        if line.trim().is_empty() || line.trim_start().starts_with('#') {
            continue;
        }
        let t = line.trim_start();
        if let Some(item) = t.strip_prefix("- ").or(if t == "-" { Some("") } else { None }) {
            if let Some(last) = out.last_mut() {
                let v = unquote(item);
                if !v.is_empty() {
                    last.1.push(v);
                }
            }
            continue;
        }
        if line.starts_with(' ') || line.starts_with('\t') {
            // a nested value: keep it with the key above, as text
            if let Some(last) = out.last_mut() {
                last.1.push(t.to_string());
            }
            continue;
        }
        if let Some((k, v)) = line.split_once(':') {
            let v = v.trim();
            let vals = if v.starts_with('[') && v.ends_with(']') {
                v[1..v.len() - 1].split(',').map(unquote).filter(|s| !s.is_empty()).collect()
            } else if v.is_empty() {
                vec![]
            } else {
                vec![unquote(v)]
            };
            out.push((k.trim().to_string(), vals));
        }
    }
    (out, body)
}

fn parse_note(root: &Path, path: &Path, text: String, mtime: SystemTime, size: u64) -> Note {
    let key = rel_key(root, path);
    let name = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let folder = key.rsplit_once('/').map(|(f, _)| f.to_string()).unwrap_or_default();
    let (front, body_start) = frontmatter(&text);
    let mut aliases = vec![];
    let mut tags: Vec<String> = vec![];
    let mut seen = HashSet::new();
    let mut add_tag = |t: &str, tags: &mut Vec<String>| {
        let t = t.trim().trim_start_matches('#');
        if !t.is_empty() && seen.insert(t.to_lowercase()) {
            tags.push(t.to_string());
        }
    };
    for (k, vals) in &front {
        match k.to_lowercase().as_str() {
            "aliases" | "alias" => {
                for v in vals {
                    aliases.extend(v.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()));
                }
            }
            "tags" | "tag" => {
                for v in vals {
                    for t in v.split([',', ' ']) {
                        add_tag(t, &mut tags);
                    }
                }
            }
            _ => {}
        }
    }
    let body = &text[body_start..];
    let masked = mask_code(body);
    for c in TAG.captures_iter(&masked) {
        add_tag(&c[1], &mut tags);
    }
    let line_of = |at: usize| masked[..at].matches('\n').count();
    let mut links = vec![];
    for c in WIKI.captures_iter(&masked) {
        let inner = &c[2];
        let target_part = inner.split('|').next().unwrap_or("");
        let (t, h) = match target_part.split_once('#') {
            Some((t, h)) => (t.trim().to_string(), Some(h.trim().to_string())),
            None => (target_part.trim().to_string(), None),
        };
        links.push(RawLink { target: t, heading: h, embed: &c[1] == "!", line: line_of(c.get(0).unwrap().start()) });
    }
    for c in MDLINK.captures_iter(&masked) {
        let dest = c[2].trim();
        let dest = dest.strip_prefix('<').and_then(|d| d.strip_suffix('>')).unwrap_or_else(|| {
            // [text](dest "title")
            dest.split_once(" \"").map(|(d, _)| d).unwrap_or(dest)
        });
        if is_url(dest) || dest.starts_with('#') {
            continue;
        }
        let dest = url_dec(dest);
        let (t, h) = match dest.split_once('#') {
            Some((t, h)) => (t.to_string(), Some(h.to_string())),
            None => (dest.clone(), None),
        };
        links.push(RawLink { target: t, heading: h, embed: &c[1] == "!", line: line_of(c.get(0).unwrap().start()) });
    }
    let lower = text.to_lowercase();
    Note { key, name, folder, text, lower, mtime, size, front, body_start, aliases, tags, links }
}

/// Collapse "a/./b/../c" to "a/c".
fn normalise(p: &str) -> String {
    let mut out: Vec<&str> = vec![];
    for part in p.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                out.pop();
            }
            x => out.push(x),
        }
    }
    out.join("/")
}

fn strip_md(s: &str) -> &str {
    if s.len() > 3 && s[s.len() - 3..].eq_ignore_ascii_case(".md") { &s[..s.len() - 3] } else { s }
}

impl Vault {
    pub fn open(root: &Path) -> Vault {
        let root = root.to_path_buf();
        let name = root
            .canonicalize()
            .ok()
            .and_then(|p| p.file_name().map(|s| s.to_string_lossy().into_owned()))
            .or_else(|| root.file_name().map(|s| s.to_string_lossy().into_owned()))
            .unwrap_or_else(|| "vault".into());
        let mut v = Vault { root, name, ..Vault::default() };
        v.rescan();
        v
    }

    /// Walk the folder again. Notes whose size and modified time haven't
    /// changed are kept as they are, so this is cheap to call on every change.
    /// Returns true if anything changed.
    pub fn rescan(&mut self) -> bool {
        let mut old: HashMap<String, Note> = self.notes.drain(..).map(|n| (n.key.clone(), n)).collect();
        let old_count = old.len();
        let mut notes = vec![];
        let mut files = vec![];
        let mut changed = false;
        let mut stack = vec![self.root.clone()];
        while let Some(dir) = stack.pop() {
            let Ok(rd) = std::fs::read_dir(&dir) else { continue };
            for e in rd.flatten() {
                let fname = e.file_name().to_string_lossy().into_owned();
                if hidden(&fname) {
                    continue;
                }
                let path = e.path();
                let Ok(ft) = e.file_type() else { continue };
                if ft.is_dir() {
                    stack.push(path);
                    continue;
                }
                // a symlink to a file still counts; symlinked folders are skipped
                let Ok(md) = std::fs::metadata(&path) else { continue };
                if !md.is_file() {
                    continue;
                }
                if !is_note(&path) {
                    files.push(rel_key(&self.root, &path));
                    continue;
                }
                let key = rel_key(&self.root, &path);
                let mtime = md.modified().unwrap_or(SystemTime::UNIX_EPOCH);
                if let Some(n) = old.remove(&key)
                    && n.mtime == mtime
                    && n.size == md.len()
                {
                    notes.push(n);
                    continue;
                }
                changed = true;
                let text = std::fs::read(&path).map(|b| String::from_utf8_lossy(&b).into_owned()).unwrap_or_default();
                // Windows line endings read the same as Unix ones
                let text = if text.contains('\r') { text.replace("\r\n", "\n") } else { text };
                notes.push(parse_note(&self.root, &path, text, mtime, md.len()));
            }
        }
        if !old.is_empty() || notes.len() != old_count {
            changed = true;
        }
        files.sort();
        if files != self.files {
            changed = true;
        }
        notes.sort_by_key(|a| a.key.to_lowercase());
        self.notes = notes;
        self.files = files;
        self.index();
        if changed {
            self.generation += 1;
        }
        changed
    }

    fn index(&mut self) {
        self.by_key.clear();
        self.by_path.clear();
        self.by_name.clear();
        self.by_alias.clear();
        for (i, n) in self.notes.iter().enumerate() {
            self.by_key.insert(n.key.clone(), i);
            self.by_path.insert(strip_md(&n.key).to_lowercase(), i);
            self.by_name.entry(n.name.to_lowercase()).or_default().push(i);
            for a in &n.aliases {
                self.by_alias.entry(a.to_lowercase()).or_default().push(i);
            }
        }
        self.files_by_name.clear();
        self.files_by_path.clear();
        for f in &self.files {
            let base = f.rsplit('/').next().unwrap_or(f).to_lowercase();
            self.files_by_name.entry(base).or_default().push(f.clone());
            self.files_by_path.insert(f.to_lowercase(), f.clone());
        }
        // folders and their counts
        let mut count: HashMap<String, usize> = HashMap::new();
        for n in &self.notes {
            let mut f = n.folder.as_str();
            while !f.is_empty() {
                *count.entry(f.to_string()).or_default() += 1;
                f = f.rsplit_once('/').map(|(a, _)| a).unwrap_or("");
            }
        }
        let mut folders: Vec<String> = count.keys().cloned().collect();
        folders.sort_by_key(|a| a.to_lowercase());
        self.folders = folders;
        self.folder_count = count;
        // tags
        self.tags.clear();
        for n in &self.notes {
            for t in &n.tags {
                let e = self.tags.entry(t.to_lowercase()).or_insert_with(|| (t.clone(), vec![]));
                e.1.push(n.key.clone());
            }
        }
        // backlinks
        let mut back: HashMap<String, Vec<(String, usize)>> = HashMap::new();
        for n in &self.notes {
            let mut seen = HashSet::new();
            for l in &n.links {
                if let Some(t) = self.resolve(&l.target, Some(&n.key))
                    && self.notes[t].key != n.key
                    && seen.insert((t, l.line))
                {
                    back.entry(self.notes[t].key.clone()).or_default().push((n.key.clone(), l.line));
                }
            }
        }
        self.backlinks = back;
    }

    pub fn note(&self, key: &str) -> Option<&Note> {
        self.by_key.get(key).map(|&i| &self.notes[i])
    }

    pub fn path_of(&self, key: &str) -> PathBuf {
        key_path(&self.root, key)
    }

    /// Where a link goes, the way Obsidian decides: an exact path from the
    /// vault root or from the linking note's folder, then a note of that name
    /// (nearest the linking note, else the shortest path), then an alias.
    pub fn resolve(&self, target: &str, from: Option<&str>) -> Option<usize> {
        let t = target.trim().replace('\\', "/");
        if t.is_empty() {
            return from.and_then(|f| self.by_key.get(f).copied());
        }
        let t = strip_md(&t).to_string();
        let lower = t.to_lowercase();
        let from_folder = from.and_then(|f| f.rsplit_once('/').map(|(d, _)| d.to_string())).unwrap_or_default();
        if lower.contains('/') {
            let root_rel = normalise(&lower);
            if let Some(&i) = self.by_path.get(&root_rel) {
                return Some(i);
            }
            let rel = normalise(&format!("{}/{}", from_folder.to_lowercase(), lower));
            if let Some(&i) = self.by_path.get(&rel) {
                return Some(i);
            }
            // a partial path: the end of some note's path
            let tail = format!("/{root_rel}");
            let mut best: Option<usize> = None;
            for (p, &i) in &self.by_path {
                if p.ends_with(&tail) && best.is_none_or(|b| self.notes[i].key.len() < self.notes[b].key.len()) {
                    best = Some(i);
                }
            }
            return best;
        }
        if let Some(cands) = self.by_name.get(&lower) {
            return Some(self.nearest(cands, &from_folder));
        }
        if let Some(cands) = self.by_alias.get(&lower) {
            return Some(self.nearest(cands, &from_folder));
        }
        None
    }

    fn nearest(&self, cands: &[usize], from_folder: &str) -> usize {
        if let Some(&i) = cands.iter().find(|&&i| self.notes[i].folder == from_folder) {
            return i;
        }
        *cands.iter().min_by_key(|&&i| (self.notes[i].key.len(), self.notes[i].key.to_lowercase())).unwrap()
    }

    /// An attachment (image, PDF...) a link or embed points at.
    pub fn resolve_file(&self, target: &str, from: Option<&str>) -> Option<String> {
        let t = target.trim().replace('\\', "/");
        if t.is_empty() {
            return None;
        }
        let lower = t.to_lowercase();
        if let Some(f) = self.files_by_path.get(&normalise(&lower)) {
            return Some(f.clone());
        }
        let from_folder = from.and_then(|f| f.rsplit_once('/').map(|(d, _)| d.to_lowercase())).unwrap_or_default();
        if let Some(f) = self.files_by_path.get(&normalise(&format!("{from_folder}/{lower}"))) {
            return Some(f.clone());
        }
        let base = lower.rsplit('/').next().unwrap_or(&lower);
        self.files_by_name.get(base).and_then(|v| v.iter().min_by_key(|f| f.len()).cloned())
    }

    pub fn notes_in(&self, folder: &str) -> Vec<usize> {
        let pre = format!("{folder}/");
        (0..self.notes.len()).filter(|&i| self.notes[i].folder == folder || self.notes[i].key.starts_with(&pre)).collect()
    }

    pub fn tagged(&self, tag: &str) -> Vec<usize> {
        (0..self.notes.len()).filter(|&i| self.notes[i].has_tag(tag)).collect()
    }

    /// Full-text search. Words (or "quoted phrases") must all appear, in the
    /// title or the text; title matches rank first, then how often it's said.
    pub fn search(&self, q: &str) -> Vec<SearchHit> {
        let terms = terms(q);
        if terms.is_empty() {
            return vec![];
        }
        let mut out = vec![];
        for n in &self.notes {
            let name = n.name.to_lowercase();
            if !terms.iter().all(|t| name.contains(t.as_str()) || n.lower.contains(t.as_str())) {
                continue;
            }
            let mut score = 0;
            if terms.iter().all(|t| name.contains(t.as_str())) {
                score += 10_000;
            }
            score += terms.iter().map(|t| n.lower.matches(t.as_str()).count().min(999)).sum::<usize>();
            out.push(SearchHit { key: n.key.clone(), snippet: snippet(n, &terms), score });
        }
        out.sort_by(|a, b| b.score.cmp(&a.score).then_with(|| a.key.to_lowercase().cmp(&b.key.to_lowercase())));
        out
    }
}

/// The words and "quoted phrases" of a query, lowercased.
pub fn terms(q: &str) -> Vec<String> {
    let mut out = vec![];
    let mut rest = q.trim();
    while !rest.is_empty() {
        if let Some(r) = rest.strip_prefix('"') {
            let (phrase, after) = r.split_once('"').unwrap_or((r, ""));
            if !phrase.trim().is_empty() {
                out.push(phrase.trim().to_lowercase());
            }
            rest = after.trim_start();
        } else {
            let (w, after) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
            out.push(w.to_lowercase());
            rest = after.trim_start();
        }
    }
    out
}

/// The body line with the first match, trimmed around it.
fn snippet(n: &Note, terms: &[String]) -> String {
    let body = n.body();
    for t in terms {
        for line in body.lines() {
            if line.to_lowercase().contains(t.as_str()) {
                let clean = clean_line(line);
                let clean = clean.as_str();
                // start a little before the hit, on a char boundary
                let chars: Vec<char> = clean.chars().collect();
                let lower: Vec<char> = clean.to_lowercase().chars().collect();
                let tc: Vec<char> = t.chars().collect();
                let at = if lower.len() == chars.len() {
                    (0..lower.len()).find(|&i| lower[i..].starts_with(&tc)).unwrap_or(0)
                } else {
                    0
                };
                // start a word or two before the hit, never mid-word
                let mut from = at.saturating_sub(14);
                if from > 0 {
                    from = (from..at).find(|&i| chars[i] == ' ').map(|i| i + 1).unwrap_or(at);
                }
                let s: String = chars[from..].iter().collect();
                return if from > 0 { format!("…{s}") } else { s };
            }
        }
    }
    body.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("").to_string()
}

/// A source line made readable on its own: [[links]] become their text,
/// list, quote, heading and task markers go.
pub fn clean_line(line: &str) -> String {
    let s = WIKI.replace_all(line, |c: &regex::Captures| {
        let inner = &c[2];
        inner.rsplit('|').next().unwrap_or(inner).to_string()
    });
    let s = s.trim().trim_start_matches(['#', '>', '-', '*', '+', ' ']).trim();
    let s = s.strip_prefix("[ ] ").or(s.strip_prefix("[x] ")).unwrap_or(s);
    s.to_string()
}

/// Where each term appears in `hay`, ignoring case: (start, end) char indexes.
pub fn find_ci(hay: &str, terms: &[String]) -> Vec<(usize, usize)> {
    let lower: Vec<char> = hay.chars().map(|c| c.to_lowercase().next().unwrap_or(c)).collect();
    let mut out = vec![];
    for t in terms {
        let tc: Vec<char> = t.chars().collect();
        if tc.is_empty() {
            continue;
        }
        let mut i = 0;
        while i + tc.len() <= lower.len() {
            if lower[i..i + tc.len()] == tc[..] {
                out.push((i, i + tc.len()));
                i += tc.len();
            } else {
                i += 1;
            }
        }
    }
    out.sort();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frontmatter_lists_and_inline() {
        let (f, body) = frontmatter("---\ntitle: \"Hi\"\ntags: [a, b]\naliases:\n  - One\n  - Two\n---\nbody");
        assert_eq!(f[0], ("title".into(), vec!["Hi".into()]));
        assert_eq!(f[1].1, vec!["a", "b"]);
        assert_eq!(f[2].1, vec!["One", "Two"]);
        assert_eq!(&"---\ntitle: \"Hi\"\ntags: [a, b]\naliases:\n  - One\n  - Two\n---\nbody"[body..], "body");
    }

    #[test]
    fn no_frontmatter_without_a_closing_line() {
        assert_eq!(frontmatter("---\nnot closed").1, 0);
    }

    #[test]
    fn code_is_masked() {
        let m = mask_code("a `#no` b\n```\n#nope [[x]]\n```\n#yes");
        assert!(!m.contains("#no"));
        assert!(!m.contains("[[x]]"));
        assert!(m.contains("#yes"));
        assert_eq!(m.len(), "a `#no` b\n```\n#nope [[x]]\n```\n#yes".len());
    }

    #[test]
    fn urls_and_drive_letters() {
        assert!(is_url("https://example.com"));
        assert!(is_url("mailto:a@b.c"));
        assert!(!is_url("C:\\notes\\a.md"));
        assert!(!is_url("Folder/Note.md"));
    }

    #[test]
    fn query_terms() {
        assert_eq!(terms("Alpha \"big plan\"  beta"), vec!["alpha", "big plan", "beta"]);
    }

    #[test]
    fn find_ignores_case() {
        assert_eq!(find_ci("Plan the PLAN", &["plan".into()]), vec![(0, 4), (9, 13)]);
    }
}
