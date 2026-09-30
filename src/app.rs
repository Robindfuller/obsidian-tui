//! The app's state and everything it can do. Drawing lives in draw.rs, keys
//! and the mouse in events.rs; both end up calling the actions here.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, channel};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::draw::Hit;
use crate::input::LineEdit;
use crate::md::{Rendered, render};
use crate::rich::{Act, Line, plain, sp};
use crate::theme::{Theme, stamp};
use crate::util::{self, osc52};
use crate::vault::{SearchHit, Vault, terms};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Focus {
    None,
    Filter,
    Rail,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Sev {
    Info,
    Error,
}

pub struct Toast {
    pub id: u64,
    pub title: String,
    pub msg: String,
    pub sev: Sev,
    pub at: Instant,
}

/// A small menu: right-click on a note, or the keys list.
pub struct Modal {
    pub title: String,
    /// None is a separator; a key of "-none" is shown but can't be picked
    pub opts: Vec<Option<(String, Line)>>,
    pub x: i32,
    pub y: i32,
    pub hi: Option<usize>,
    pub top: usize,
    /// what the menu is about (a note key)
    pub ctx: String,
}

#[derive(Clone, Debug)]
pub struct Row {
    pub key: String,
    /// a search result's matching line
    pub snippet: Option<String>,
}

#[derive(Clone, Debug)]
pub struct RailEntry {
    /// None for a section label
    pub key: Option<String>,
    pub label: String,
    pub short: String,
    pub count: String,
    pub depth: usize,
    /// Some(open) for a folder with folders inside
    pub fold: Option<bool>,
}

#[derive(Clone, Copy, PartialEq)]
enum Nav {
    List,
    Link,
    History,
}

pub struct App {
    pub vault: Vault,
    pub t: Theme,
    theme_stamp: Vec<u128>,
    theme_gen: u64,
    theme_at: Instant,
    pub size: (u16, u16),
    /// the sidebar entry being shown: "all", "recent", "search", "folder:..", "tag:.."
    pub view: String,
    /// the last full-text search, and what it found
    pub query: String,
    pub results: Vec<SearchHit>,
    /// words to mark in the open note (it was opened from a search)
    pub terms: Vec<String>,
    pub filter: LineEdit,
    pub focus: Focus,
    pub rail_collapsed: bool,
    pub rail_scroll: usize,
    pub rail_cursor: usize,
    pub open_folders: HashSet<String>,
    pub rows: Vec<Row>,
    pub sel: Option<String>,
    pub list_scroll: usize,
    pub scroll_sel: bool,
    pub note: Option<String>,
    pub note_scroll: usize,
    /// rows of note body on screen at the last draw
    pub note_h: usize,
    pub tab: usize,
    cache: Option<RenderCache>,
    pub history: Vec<(String, usize)>,
    pub hist_pos: usize,
    last_nav: Option<Nav>,
    pub link_sel: Option<usize>,
    pub hit_idx: usize,
    pub scroll_to: Option<ScrollTo>,
    /// narrow terminals show the list or the note, not both
    pub narrow_note: bool,
    pub sort_mod: bool,
    pub list_w: u16,
    pub modals: Vec<Modal>,
    pub toasts: Vec<Toast>,
    toast_id: u64,
    pub hits: Vec<Hit>,
    pub mouse: Option<(u16, u16)>,
    pub mouse_at: Option<Instant>,
    pub tooltip: Option<String>,
    pub quit: bool,
    pub dirty: bool,
    /// a note to hand to $EDITOR, picked up by the main loop
    pub pending_edit: Option<PathBuf>,
    /// OSC 52 clipboard writes, picked up by the main loop
    pub pending_out: Vec<String>,
    /// what was opened outside the app (tests look here)
    pub opened: Vec<String>,
    pub headless: bool,
    watch_rx: Option<Receiver<()>>,
    _watcher: Option<notify::RecommendedWatcher>,
    rescan_at: Option<Instant>,
    poll_at: Option<Instant>,
    state_path: PathBuf,
}

/// The last render: note key, width, vault and theme generations, search
/// terms, and the result.
type RenderCache = (String, usize, u64, u64, Vec<String>, std::rc::Rc<Rendered>);

#[derive(Clone, Debug, PartialEq)]
pub enum ScrollTo {
    Heading(String),
    Hit,
    Line(usize),
}

pub fn state_file() -> PathBuf {
    if let Ok(p) = std::env::var("OBSIDIAN_TUI_STATE") {
        return PathBuf::from(p);
    }
    let base = if cfg!(target_os = "windows") {
        std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(util::home)
    } else {
        std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).unwrap_or_else(|| util::home().join(".config"))
    };
    base.join("obsidian-tui").join("state.json")
}

fn slug(s: &str) -> String {
    s.to_lowercase().chars().filter(|c| c.is_alphanumeric()).collect()
}

impl App {
    pub fn new(root: &Path) -> App {
        App::with_state(root, state_file())
    }

    pub fn with_state(root: &Path, state_path: PathBuf) -> App {
        let vault = Vault::open(root);
        let t = Theme::load();
        let mut app = App {
            vault,
            t,
            theme_stamp: stamp(),
            theme_gen: 0,
            theme_at: Instant::now(),
            size: (120, 40),
            view: "all".into(),
            query: String::new(),
            results: vec![],
            terms: vec![],
            filter: LineEdit::default(),
            focus: Focus::None,
            rail_collapsed: false,
            rail_scroll: 0,
            rail_cursor: 0,
            open_folders: HashSet::new(),
            rows: vec![],
            sel: None,
            list_scroll: 0,
            scroll_sel: false,
            note: None,
            note_scroll: 0,
            note_h: 20,
            tab: 0,
            cache: None,
            history: vec![],
            hist_pos: 0,
            last_nav: None,
            link_sel: None,
            hit_idx: 0,
            scroll_to: None,
            narrow_note: false,
            sort_mod: false,
            list_w: 42,
            modals: vec![],
            toasts: vec![],
            toast_id: 0,
            hits: vec![],
            mouse: None,
            mouse_at: None,
            tooltip: None,
            quit: false,
            dirty: true,
            pending_edit: None,
            pending_out: vec![],
            opened: vec![],
            headless: false,
            watch_rx: None,
            _watcher: None,
            rescan_at: None,
            poll_at: None,
            state_path,
        };
        app.load_state();
        app
    }

    fn vault_id(&self) -> String {
        self.vault.root.canonicalize().unwrap_or_else(|_| self.vault.root.clone()).to_string_lossy().into_owned()
    }

    fn load_state(&mut self) {
        let st: Value = std::fs::read_to_string(&self.state_path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or(json!({}));
        self.rail_collapsed = st["rail_collapsed"].as_bool().unwrap_or(false);
        self.sort_mod = st["sort_mod"].as_bool().unwrap_or(false);
        self.list_w = st["list_w"].as_u64().map(|v| v as u16).unwrap_or(42);
        let v = &st["vaults"][self.vault_id()];
        if let Some(open) = v["open"].as_array() {
            self.open_folders = open.iter().filter_map(|x| x.as_str().map(str::to_string)).collect();
        }
        let view = v["view"].as_str().unwrap_or("all").to_string();
        let valid = match view.split_once(':') {
            Some(("folder", f)) => self.vault.folder_count.contains_key(f),
            Some(("tag", t)) => self.vault.tags.contains_key(t),
            _ => view == "all" || view == "recent",
        };
        self.view = if valid { view } else { "all".into() };
        self.refresh_rows();
        let note = v["note"].as_str().map(str::to_string).filter(|k| self.vault.note(k).is_some());
        match note {
            Some(k) => self.open_note(&k, Nav::List),
            None => self.select_first(),
        }
    }

    pub fn save_state(&self) {
        // a headless --dump never touches the real state file
        if self.headless && self.state_path == state_file() {
            return;
        }
        let mut st: Value = std::fs::read_to_string(&self.state_path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or(json!({}));
        if !st.is_object() {
            st = json!({});
        }
        st["rail_collapsed"] = json!(self.rail_collapsed);
        st["sort_mod"] = json!(self.sort_mod);
        st["list_w"] = json!(self.list_w);
        if !st["vaults"].is_object() {
            st["vaults"] = json!({});
        }
        let mut open: Vec<&String> = self.open_folders.iter().collect();
        open.sort();
        let view = if self.view == "search" { "all" } else { self.view.as_str() };
        st["vaults"][self.vault_id()] = json!({"view": view, "note": self.note, "open": open});
        if let Some(dir) = self.state_path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let tmp = self.state_path.with_extension("tmp");
        if std::fs::write(&tmp, serde_json::to_string_pretty(&st).unwrap_or_default()).is_ok() {
            let _ = std::fs::rename(&tmp, &self.state_path);
        }
    }

    /// Start watching the vault for changes (not in tests).
    pub fn start(&mut self) {
        if self.headless {
            return;
        }
        use notify::{RecursiveMode, Watcher};
        let (tx, rx) = channel();
        // events come with absolute paths; the vault may have been given as "."
        let roots: Vec<PathBuf> = [std::path::absolute(&self.vault.root).ok(), self.vault.root.canonicalize().ok()]
            .into_iter()
            .flatten()
            .collect();
        let w = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
            if let Ok(ev) = res {
                // ignore .obsidian (Obsidian writes its workspace file often), .git, .trash
                let counts = ev.paths.iter().any(|p| match roots.iter().find_map(|r| p.strip_prefix(r).ok()) {
                    Some(rel) => !rel.components().any(|c| c.as_os_str().to_string_lossy().starts_with('.')),
                    None => true,
                });
                if counts {
                    let _ = tx.send(());
                }
            }
        });
        match w {
            Ok(mut w) => {
                if w.watch(&self.vault.root, RecursiveMode::Recursive).is_ok() {
                    self._watcher = Some(w);
                    self.watch_rx = Some(rx);
                } else {
                    self.poll_at = Some(Instant::now() + Duration::from_secs(3));
                }
            }
            Err(_) => self.poll_at = Some(Instant::now() + Duration::from_secs(3)),
        }
    }

    /// Timers: toasts, theme switches, vault changes.
    pub fn tick(&mut self) {
        let before = self.toasts.len();
        self.toasts.retain(|t| t.at.elapsed() < Duration::from_secs(if t.sev == Sev::Error { 6 } else { 4 }));
        if self.toasts.len() != before {
            self.dirty = true;
        }
        if self.theme_at.elapsed() > Duration::from_millis(1500) {
            self.theme_at = Instant::now();
            self.check_theme();
        }
        if let Some(rx) = &self.watch_rx {
            let mut any = false;
            while rx.try_recv().is_ok() {
                any = true;
            }
            if any {
                self.rescan_at = Some(Instant::now() + Duration::from_millis(250));
            }
        }
        if self.rescan_at.is_some_and(|t| Instant::now() >= t) {
            self.rescan_at = None;
            self.rescan_now();
        }
        if self.poll_at.is_some_and(|t| Instant::now() >= t) {
            self.poll_at = Some(Instant::now() + Duration::from_secs(3));
            self.rescan_now();
        }
    }

    pub fn check_theme(&mut self) {
        let s = stamp();
        if s != self.theme_stamp {
            self.theme_stamp = s;
            self.t = Theme::load();
            self.theme_gen += 1;
            self.dirty = true;
        }
    }

    pub fn rescan_now(&mut self) {
        if !self.vault.rescan() {
            return;
        }
        self.dirty = true;
        if !self.query.is_empty() {
            self.results = self.vault.search(&self.query);
        }
        if let Some(k) = self.note.clone()
            && self.vault.note(&k).is_none()
        {
            self.note = None;
            self.toast("", &format!("{} was moved or deleted.", k), Sev::Info);
        }
        let view_ok = match self.view.split_once(':') {
            Some(("folder", f)) => self.vault.folder_count.contains_key(f),
            Some(("tag", t)) => self.vault.tags.contains_key(t),
            _ => true,
        };
        if !view_ok {
            self.view = "all".into();
        }
        self.refresh_rows();
        if self.sel.as_ref().is_none_or(|s| !self.rows.iter().any(|r| &r.key == s)) {
            self.sel = self.rows.first().map(|r| r.key.clone());
        }
    }

    pub fn toast(&mut self, title: &str, msg: &str, sev: Sev) {
        self.toast_id += 1;
        self.toasts.push(Toast { id: self.toast_id, title: title.into(), msg: msg.into(), sev, at: Instant::now() });
        self.dirty = true;
    }

    // ------------------------------------------------------------ the list
    pub fn view_label(&self) -> String {
        match self.view.split_once(':') {
            Some(("folder", f)) => f.rsplit('/').next().unwrap_or(f).to_string(),
            Some(("tag", t)) => format!("#{}", self.vault.tags.get(t).map(|x| x.0.as_str()).unwrap_or(t)),
            _ => match self.view.as_str() {
                "recent" => "Recent".into(),
                "search" => format!("Search: {}", self.query),
                _ => "All notes".into(),
            },
        }
    }

    pub fn refresh_rows(&mut self) {
        let v = &self.vault;
        let mut idx: Vec<usize> = match self.view.split_once(':') {
            Some(("folder", f)) => v.notes_in(f),
            Some(("tag", t)) => v.tagged(t),
            _ => (0..v.notes.len()).collect(),
        };
        let q = self.filter.value().to_lowercase();
        let keep = |i: &usize| q.is_empty() || v.notes[*i].name.to_lowercase().contains(&q);
        let rows: Vec<Row> = if self.view == "search" {
            self.results
                .iter()
                .filter(|h| v.by_key.get(&h.key).is_some_and(keep))
                .map(|h| Row { key: h.key.clone(), snippet: Some(h.snippet.clone()) })
                .collect()
        } else {
            idx.retain(keep);
            if self.view == "recent" {
                idx.sort_by(|a, b| v.notes[*b].mtime.cmp(&v.notes[*a].mtime));
                idx.truncate(50);
            } else if self.sort_mod {
                idx.sort_by(|a, b| v.notes[*b].mtime.cmp(&v.notes[*a].mtime));
            } else {
                idx.sort_by(|a, b| {
                    v.notes[*a].name.to_lowercase().cmp(&v.notes[*b].name.to_lowercase()).then(a.cmp(b))
                });
            }
            idx.into_iter().map(|i| Row { key: v.notes[i].key.clone(), snippet: None }).collect()
        };
        self.rows = rows;
        self.dirty = true;
    }

    fn select_first(&mut self) {
        self.list_scroll = 0;
        if let Some(k) = self.rows.first().map(|r| r.key.clone()) {
            self.open_note(&k, Nav::List);
        } else {
            self.sel = None;
        }
    }

    pub fn set_view(&mut self, key: &str) {
        if key == "search" && self.query.is_empty() {
            return;
        }
        if let Some(f) = key.strip_prefix("folder:") {
            // opening a folder shows the folders inside it, and its parents
            let mut p = f;
            loop {
                self.open_folders.insert(p.to_string());
                match p.rsplit_once('/') {
                    Some((a, _)) => p = a,
                    None => break,
                }
            }
        }
        self.view = key.to_string();
        self.filter.set("");
        self.refresh_rows();
        self.select_first();
        self.save_state();
    }

    pub fn action_move(&mut self, d: i64) {
        if self.rows.is_empty() {
            return;
        }
        let i = self.sel.as_ref().and_then(|s| self.rows.iter().position(|r| &r.key == s));
        let n = self.rows.len() as i64;
        let j = match i {
            Some(i) => (i as i64 + d).clamp(0, n - 1),
            None => {
                if d > 0 {
                    0
                } else {
                    n - 1
                }
            }
        } as usize;
        let k = self.rows[j].key.clone();
        self.open_note(&k, Nav::List);
    }

    // ------------------------------------------------------------ the note
    fn open_note(&mut self, key: &str, nav: Nav) {
        if self.vault.note(key).is_none() {
            return;
        }
        if let Some(cur) = &self.note
            && let Some(e) = self.history.get_mut(self.hist_pos)
            && &e.0 == cur
        {
            e.1 = self.note_scroll;
        }
        match nav {
            Nav::History => {}
            Nav::List if self.last_nav == Some(Nav::List) && !self.history.is_empty() => {
                self.history.truncate(self.hist_pos + 1);
                self.history[self.hist_pos] = (key.to_string(), 0);
            }
            _ => {
                if !self.history.is_empty() {
                    self.history.truncate(self.hist_pos + 1);
                }
                if self.history.last().is_none_or(|e| e.0 != key) {
                    self.history.push((key.to_string(), 0));
                }
                self.hist_pos = self.history.len() - 1;
            }
        }
        if nav != Nav::History {
            self.last_nav = Some(nav);
        }
        let same = self.note.as_deref() == Some(key);
        self.note = Some(key.to_string());
        if !same || nav != Nav::List {
            self.note_scroll = 0;
            self.link_sel = None;
        }
        if nav == Nav::Link {
            self.tab = 0;
        }
        if nav != Nav::History {
            self.terms = if self.view == "search" && nav == Nav::List { terms(&self.query) } else { vec![] };
            self.hit_idx = 0;
            self.scroll_to = if self.terms.is_empty() { None } else { Some(ScrollTo::Hit) };
        }
        if self.rows.iter().any(|r| r.key == key) {
            self.sel = Some(key.to_string());
            self.scroll_sel = true;
        }
        self.dirty = true;
    }

    /// Open the selected note: in a narrow terminal this swaps the list for it.
    pub fn open_current(&mut self) {
        if let Some(i) = self.link_sel {
            let links = self.body_links();
            if let Some((_, a)) = links.get(i).cloned() {
                self.run(&a);
            }
            return;
        }
        if let Some(k) = self.sel.clone() {
            self.open_note(&k, Nav::List);
            self.narrow_note = true;
        }
    }

    pub fn back(&mut self) {
        if self.hist_pos == 0 || self.history.is_empty() {
            return;
        }
        self.save_hist_scroll();
        self.hist_pos -= 1;
        self.go_hist();
    }

    pub fn forward(&mut self) {
        if self.hist_pos + 1 >= self.history.len() {
            return;
        }
        self.save_hist_scroll();
        self.hist_pos += 1;
        self.go_hist();
    }

    fn save_hist_scroll(&mut self) {
        if let Some(e) = self.history.get_mut(self.hist_pos) {
            e.1 = self.note_scroll;
        }
    }

    fn go_hist(&mut self) {
        let (k, s) = self.history[self.hist_pos].clone();
        self.open_note(&k, Nav::History);
        self.note_scroll = s;
        self.link_sel = None;
        self.terms.clear();
        self.last_nav = Some(Nav::History);
        self.narrow_note = true;
    }

    /// The open note, drawn at `width` (cached until the note, the vault or
    /// the theme changes).
    pub fn rendered(&mut self, width: usize) -> Option<std::rc::Rc<Rendered>> {
        let key = self.note.clone()?;
        if let Some((k, w, g, tg, terms, r)) = &self.cache
            && *k == key
            && *w == width
            && *g == self.vault.generation
            && *tg == self.theme_gen
            && *terms == self.terms
        {
            return Some(r.clone());
        }
        let note = self.vault.note(&key)?;
        let r = std::rc::Rc::new(render(note, &self.vault, &self.t, width, &self.terms));
        self.cache = Some((key, width, self.vault.generation, self.theme_gen, self.terms.clone(), r.clone()));
        Some(r)
    }

    /// The lines of whichever tab is showing, and the links in them.
    pub fn body(&mut self, width: usize) -> (Vec<Line>, Vec<(usize, Act)>, Vec<usize>) {
        let Some(key) = self.note.clone() else { return (vec![], vec![], vec![]) };
        match self.tab {
            1 => {
                let lines = self.links_tab(&key, width);
                let links = collect_links(&lines);
                (lines, links, vec![])
            }
            2 => {
                let lines = self.outline_tab(width);
                let links = collect_links(&lines);
                (lines, links, vec![])
            }
            _ => match self.rendered(width) {
                Some(r) => (r.lines.clone(), r.links.clone(), r.hits.clone()),
                None => (vec![], vec![], vec![]),
            },
        }
    }

    pub fn body_width(&self) -> usize {
        let g = self.geo();
        g.note.map(|r| r.width.saturating_sub(5) as usize).unwrap_or(40).max(8)
    }

    pub fn body_links(&mut self) -> Vec<(usize, Act)> {
        let w = self.body_width();
        self.body(w).1
    }

    fn links_tab(&mut self, key: &str, width: usize) -> Vec<Line> {
        let (faint, dim, acc) = (self.t.c("faint"), self.t.c("dim"), self.t.c("accent"));
        let mut out: Vec<Line> = vec![];
        let back = self.vault.backlinks.get(key).cloned().unwrap_or_default();
        out.push(vec![sp(format!("LINKED MENTIONS · {}", back.len()), faint).b()]);
        out.push(vec![]);
        if back.is_empty() {
            out.push(vec![sp("No other note links here.", dim)]);
        }
        let mut last = String::new();
        for (src, line) in &back {
            let Some(n) = self.vault.note(src) else { continue };
            if *src != last {
                if !last.is_empty() {
                    out.push(vec![]);
                }
                let mut l = vec![sp(n.name.clone(), acc).on(Act::Open { key: src.clone(), heading: None })];
                if !n.folder.is_empty() {
                    l.push(sp(format!("  {}", n.folder), faint));
                }
                out.push(l);
                last = src.clone();
            }
            let ctx = crate::vault::clean_line(n.body_line(*line));
            let ctx = util::crop(&ctx, width.saturating_sub(2) * 2);
            for w in crate::rich::wrap(&[sp(ctx, dim)], width.saturating_sub(2)) {
                let mut l = vec![plain("  ")];
                l.extend(w);
                out.push(l);
            }
        }
        // links out of this note
        let Some(n) = self.vault.note(key) else { return out };
        let mut seen = HashSet::new();
        let mut outgoing: Vec<Line> = vec![];
        for l in &n.links {
            let (label, act, col) = match self.vault.resolve(&l.target, Some(key)) {
                Some(i) => {
                    let k = self.vault.notes[i].key.clone();
                    (self.vault.notes[i].name.clone(), Act::Open { key: k, heading: l.heading.clone() }, acc)
                }
                None => match self.vault.resolve_file(&l.target, Some(key)) {
                    Some(f) => (f.rsplit('/').next().unwrap_or(&f).to_string(), Act::File(f.clone()), self.t.c("sea")),
                    None => (format!("{}  (no such note)", l.target), Act::Missing(l.target.clone()), faint),
                },
            };
            if l.target.is_empty() || !seen.insert(label.to_lowercase()) {
                continue;
            }
            outgoing.push(vec![sp("• ", faint), sp(label, col).on(act)]);
        }
        out.push(vec![]);
        out.push(vec![sp(format!("LINKS OUT · {}", outgoing.len()), faint).b()]);
        out.push(vec![]);
        if outgoing.is_empty() {
            out.push(vec![sp("This note doesn't link anywhere.", dim)]);
        }
        out.extend(outgoing);
        out
    }

    fn outline_tab(&mut self, width: usize) -> Vec<Line> {
        let w = self.body_width();
        let Some(r) = self.rendered(w) else { return vec![] };
        let mut out: Vec<Line> = vec![];
        if r.headings.is_empty() {
            out.push(vec![sp("This note has no headings.", self.t.c("dim"))]);
        }
        for (level, text, line) in &r.headings {
            let col = match level {
                1 => self.t.c("accent"),
                2 => self.t.c("s2"),
                3 => self.t.c("s3"),
                _ => self.t.c("dim"),
            };
            let indent = "  ".repeat((*level as usize).saturating_sub(1));
            let text = util::crop(text, width.saturating_sub(indent.len()));
            out.push(vec![plain(indent), sp(text, col).b_if(*level <= 2).on(Act::Line(*line))]);
        }
        out
    }

    /// Keep the chosen link on screen.
    fn show_link(&mut self) {
        let links = self.body_links();
        if let Some((line, _)) = self.link_sel.and_then(|i| links.get(i)) {
            let h = self.note_h.max(1);
            if *line < self.note_scroll {
                self.note_scroll = line.saturating_sub(1);
            } else if *line >= self.note_scroll + h {
                self.note_scroll = line + 2 - h.min(line + 2);
            }
        }
    }

    pub fn next_link(&mut self, d: i64) {
        let links = self.body_links();
        if links.is_empty() {
            return;
        }
        let n = links.len() as i64;
        let i = match self.link_sel {
            Some(i) => (i as i64 + d).rem_euclid(n),
            None => {
                // start from the first link on screen
                let first = links.iter().position(|(l, _)| *l >= self.note_scroll).unwrap_or(0) as i64;
                if d > 0 { first } else { (first - 1).rem_euclid(n) }
            }
        };
        self.link_sel = Some(i as usize);
        self.show_link();
    }

    pub fn next_hit(&mut self, d: i64) {
        let w = self.body_width();
        let hits = self.body(w).2;
        if hits.is_empty() {
            self.toast("", "No search matches in this note.", Sev::Info);
            return;
        }
        let n = hits.len() as i64;
        self.hit_idx = (self.hit_idx as i64 + d).rem_euclid(n) as usize;
        self.note_scroll = hits[self.hit_idx].saturating_sub(2);
    }

    pub fn scroll_note(&mut self, d: i64) {
        self.note_scroll = (self.note_scroll as i64 + d).max(0) as usize;
        self.dirty = true;
    }

    /// Resolve a pending scroll now that the note is drawn at a width.
    pub fn apply_scroll_to(&mut self, width: usize) {
        let Some(st) = self.scroll_to.take() else { return };
        let Some(r) = self.rendered(width) else { return };
        match st {
            ScrollTo::Heading(h) => {
                let want = slug(&h);
                if let Some((_, _, line)) = r.headings.iter().find(|(_, t, _)| slug(t) == want) {
                    self.note_scroll = *line;
                }
            }
            ScrollTo::Hit => {
                if let Some(l) = r.hits.first() {
                    self.note_scroll = l.saturating_sub(2);
                }
            }
            ScrollTo::Line(l) => self.note_scroll = l,
        }
    }

    // ------------------------------------------------------------ search
    pub fn run_search(&mut self) {
        let q = self.filter.value().trim().to_string();
        if q.is_empty() {
            return;
        }
        self.query = q;
        self.results = self.vault.search(&self.query);
        self.view = "search".into();
        self.filter.set("");
        self.focus = Focus::None;
        self.refresh_rows();
        self.select_first();
        if self.results.is_empty() {
            self.toast("", &format!("Nothing in the vault says “{}”.", self.query), Sev::Info);
        }
    }

    pub fn clear_search(&mut self) {
        self.query.clear();
        self.results.clear();
        self.terms.clear();
        if self.view == "search" {
            self.set_view("all");
        }
    }

    // ------------------------------------------------------------ sidebar
    pub fn rail_entries(&self) -> Vec<RailEntry> {
        let v = &self.vault;
        let e = |key: &str, label: String, short: &str, count: usize, depth, fold| RailEntry {
            key: Some(key.into()),
            label,
            short: short.into(),
            count: count.to_string(),
            depth,
            fold,
        };
        let mut out = vec![
            e("all", "All notes".into(), "All", v.notes.len(), 0, None),
            e("recent", "Recent".into(), "Rec", v.notes.len().min(50), 0, None),
        ];
        if !self.query.is_empty() {
            let label = if self.query.starts_with('"') { self.query.clone() } else { format!("“{}”", self.query) };
            out.push(e("search", label, "Find", self.results.len(), 0, None));
        }
        if !v.folders.is_empty() {
            out.push(RailEntry { key: None, label: "Folders".into(), short: String::new(), count: String::new(), depth: 0, fold: None });
            let has_kids = |f: &str| {
                let pre = format!("{f}/");
                v.folders.iter().any(|x| x.starts_with(&pre))
            };
            for f in &v.folders {
                // shown if every folder above it is open
                let mut shown = true;
                let mut p = f.as_str();
                while let Some((a, _)) = p.rsplit_once('/') {
                    if !self.open_folders.contains(a) {
                        shown = false;
                        break;
                    }
                    p = a;
                }
                if !shown {
                    continue;
                }
                let name = f.rsplit('/').next().unwrap_or(f);
                let depth = f.matches('/').count();
                let fold = if has_kids(f) { Some(self.open_folders.contains(f)) } else { None };
                let short: String = name.chars().take(3).collect();
                out.push(e(&format!("folder:{f}"), name.to_string(), &short, v.folder_count[f], depth, fold));
            }
        }
        if !v.tags.is_empty() {
            out.push(RailEntry { key: None, label: "Tags".into(), short: String::new(), count: String::new(), depth: 0, fold: None });
            for (lower, (disp, keys)) in &v.tags {
                let short: String = format!("#{}", disp.chars().take(2).collect::<String>());
                out.push(e(&format!("tag:{lower}"), format!("#{disp}"), &short, keys.len(), 0, None));
            }
        }
        out
    }

    pub fn toggle_fold(&mut self, f: &str) {
        if !self.open_folders.remove(f) {
            self.open_folders.insert(f.to_string());
        }
        self.dirty = true;
        self.save_state();
    }

    pub fn rail_move(&mut self, d: i64) {
        let keys: Vec<String> = self.rail_entries().into_iter().filter_map(|e| e.key).collect();
        if keys.is_empty() {
            return;
        }
        let cur = keys.iter().position(|k| *k == self.view).unwrap_or(0) as i64;
        let j = (cur + d).clamp(0, keys.len() as i64 - 1) as usize;
        let k = keys[j].clone();
        self.rail_cursor = j;
        self.set_view(&k);
        self.focus = Focus::Rail;
    }

    pub fn toggle_rail(&mut self) {
        self.rail_collapsed = !self.rail_collapsed;
        self.dirty = true;
        self.save_state();
    }

    // ------------------------------------------------------------ outside
    fn open_outside(&mut self, target: &str, what: &str) {
        if self.headless {
            self.opened.push(target.to_string());
            return;
        }
        if util::open_external(target) {
            self.toast("", &format!("Opened {what}."), Sev::Info);
        } else {
            self.toast("Couldn't open it", target, Sev::Error);
        }
    }

    pub fn edit(&mut self, key: Option<String>) {
        let Some(k) = key.or(self.note.clone()).or(self.sel.clone()) else { return };
        self.pending_edit = Some(self.vault.path_of(&k));
    }

    fn obsidian_url(&self, key: &str) -> String {
        let file = key.strip_suffix(".md").unwrap_or(key);
        format!("obsidian://open?vault={}&file={}", util::url_enc(&self.vault.name), util::url_enc(file))
    }

    fn copy(&mut self, text: String, what: &str) {
        self.pending_out.push(osc52(&text));
        self.toast("", &format!("Copied {what}."), Sev::Info);
    }

    /// Run a clicked (or keyed) action.
    pub fn run(&mut self, a: &Act) {
        self.dirty = true;
        match a {
            Act::ToggleRail => self.toggle_rail(),
            Act::Rail(k) => {
                self.focus = Focus::None;
                self.set_view(k);
            }
            Act::Fold(f) => self.toggle_fold(f),
            Act::Open { key, heading } => {
                self.open_note(key, Nav::Link);
                self.narrow_note = true;
                if let Some(h) = heading {
                    self.scroll_to = Some(ScrollTo::Heading(h.clone()));
                }
            }
            Act::Missing(t) => self.toast("", &format!("There's no note called “{t}” in this vault yet."), Sev::Info),
            Act::Url(u) => {
                let u = u.clone();
                self.open_outside(&u, "the link");
            }
            Act::File(f) => {
                let p = self.vault.path_of(f).to_string_lossy().into_owned();
                self.open_outside(&p, "the file");
            }
            Act::Tag(t) => {
                let k = format!("tag:{}", t.to_lowercase());
                if self.vault.tags.contains_key(&t.to_lowercase()) {
                    self.set_view(&k);
                }
            }
            Act::Tab(i) => {
                self.tab = *i;
                self.note_scroll = 0;
                self.link_sel = None;
            }
            Act::Line(l) => {
                self.tab = 0;
                self.note_scroll = *l;
                self.link_sel = None;
            }
            Act::Back => self.back(),
            Act::Forward => self.forward(),
            Act::Edit => self.edit(None),
            Act::Obsidian => {
                if let Some(k) = self.note.clone() {
                    let u = self.obsidian_url(&k);
                    self.open_outside(&u, "it in Obsidian");
                }
            }
            Act::CopyPath => {
                if let Some(k) = self.note.clone() {
                    let p = self.vault.path_of(&k).to_string_lossy().into_owned();
                    self.copy(p, "the path");
                }
            }
            Act::CopyLink => {
                if let Some(n) = self.note.clone().and_then(|k| self.vault.note(&k).map(|n| n.name.clone())) {
                    self.copy(format!("[[{n}]]"), "the link");
                }
            }
            Act::Filter => self.focus = Focus::Filter,
            Act::Search => self.run_search(),
            Act::ClearSearch => self.clear_search(),
            Act::Sort => {
                self.sort_mod = !self.sort_mod;
                self.refresh_rows();
                self.scroll_sel = true;
                self.save_state();
            }
            Act::NextHit => self.next_hit(1),
            Act::PrevHit => self.next_hit(-1),
            Act::Help => self.help(),
            Act::Quit => self.quit = true,
            Act::MenuPick(_) | Act::Toast(_) => {}
        }
    }

    // ------------------------------------------------------------ menus
    pub fn row_menu(&mut self, key: &str, x: i32, y: i32) {
        let (ink, faint) = (self.t.c("ink"), self.t.c("faint"));
        let o = |k: &str, label: &str, hint: &str| {
            Some((k.to_string(), vec![sp(util::fit(label, 22), ink), sp(util::rjust(hint, 5), faint)]))
        };
        let name = self.vault.note(key).map(|n| n.name.clone()).unwrap_or_default();
        self.modals.push(Modal {
            title: name,
            opts: vec![
                o("open", "Open", "enter"),
                o("edit", "Open in your editor", "e"),
                o("obsidian", "Open in Obsidian", "o"),
                None,
                o("copylink", "Copy [[link]]", "y"),
                o("copypath", "Copy path", "Y"),
                o("folder", "Show its folder", ""),
            ],
            x,
            y,
            hi: None,
            top: 0,
            ctx: key.to_string(),
        });
    }

    pub fn help(&mut self) {
        let (ink, faint) = (self.t.c("ink"), self.t.c("faint"));
        let keys = [
            ("↑ ↓  j k", "move through the notes"),
            ("enter", "open the note (or the chosen link)"),
            ("tab  shift+tab", "step through links in the note"),
            ("/", "filter titles; enter searches every note"),
            ("n  N", "next / previous search match"),
            ("[  ]  alt+← →", "back / forward"),
            ("← h", "move to the sidebar (→ to come back)"),
            ("1 2 3", "note, links, outline"),
            ("space  pgup pgdn", "scroll the note"),
            ("J K  home end", "scroll a line / to the ends"),
            ("e", "edit in $EDITOR"),
            ("o", "open in Obsidian"),
            ("y  Y", "copy [[link]] / path"),
            ("s", "sort by name or date"),
            ("b", "fold the sidebar"),
            ("{  }", "narrower / wider list"),
            ("r", "read the vault again"),
            ("q", "quit"),
        ];
        let opts = keys
            .iter()
            .map(|(k, d)| Some(("-none".to_string(), vec![sp(util::fit(k, 18), ink), sp(d.to_string(), faint)])))
            .collect();
        let (w, h) = self.size;
        self.modals.push(Modal {
            title: "Keys".into(),
            opts,
            x: (w as i32 - 62) / 2,
            y: (h as i32 - 22).max(0) / 2,
            hi: None,
            top: 0,
            ctx: String::new(),
        });
    }

    pub fn menu_pick(&mut self, key: Option<String>) {
        let Some(m) = self.modals.pop() else { return };
        let Some(k) = key else { return };
        let ctx = m.ctx;
        if !ctx.is_empty() && self.note.as_deref() != Some(ctx.as_str()) {
            self.open_note(&ctx, Nav::List);
        }
        match k.as_str() {
            "open" => {
                self.open_note(&ctx, Nav::List);
                self.narrow_note = true;
            }
            "edit" => self.edit(Some(ctx)),
            "obsidian" => self.run(&Act::Obsidian),
            "copylink" => self.run(&Act::CopyLink),
            "copypath" => self.run(&Act::CopyPath),
            "folder" => {
                let f = self.vault.note(&ctx).map(|n| n.folder.clone()).unwrap_or_default();
                let v = if f.is_empty() { "all".to_string() } else { format!("folder:{f}") };
                self.set_view(&v);
                self.open_note(&ctx, Nav::List);
            }
            _ => {}
        }
    }

    pub fn escape(&mut self) {
        if self.link_sel.is_some() {
            self.link_sel = None;
        } else if self.focus == Focus::Rail {
            self.focus = Focus::None;
        } else if self.narrow_note && self.narrow() {
            self.narrow_note = false;
        } else if !self.filter.value().is_empty() {
            self.filter.set("");
            self.refresh_rows();
        } else if self.view == "search" {
            self.clear_search();
        }
    }

    pub fn narrow(&self) -> bool {
        self.size.0 < 100
    }

    pub fn set_list_width(&mut self, d: i64) {
        self.list_w = (self.list_w as i64 + d).clamp(28, 90) as u16;
        self.save_state();
    }
}

fn collect_links(lines: &[Line]) -> Vec<(usize, Act)> {
    let mut out = vec![];
    for (i, l) in lines.iter().enumerate() {
        let mut last: Option<&Act> = None;
        for s in l {
            if s.act.is_some() && s.act.as_ref() != last {
                out.push((i, s.act.clone().unwrap()));
            }
            last = s.act.as_ref();
        }
    }
    out
}
