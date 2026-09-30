//! Keys, clicks and the wheel. Up and down always move through the notes,
//! whatever was last clicked or scrolled; only the filter box (and the
//! sidebar, when you move there on purpose with ←) changes what keys do.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};

use crate::app::{App, Focus, Stage};
use crate::draw::{HitKind, Scroll};
use crate::input::Ed;
use crate::rich::Act;

impl App {
    pub fn on_key(&mut self, k: KeyEvent) {
        if k.kind == KeyEventKind::Release {
            return;
        }
        self.dirty = true;
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        let alt = k.modifiers.contains(KeyModifiers::ALT);
        if ctrl && matches!(k.code, KeyCode::Char('c') | KeyCode::Char('q')) {
            self.quit = true;
            return;
        }
        if !self.modals.is_empty() {
            self.menu_key(k);
            return;
        }
        if self.focus == Focus::Filter {
            match k.code {
                KeyCode::Esc => {
                    if self.filter.value().is_empty() {
                        self.focus = Focus::None;
                    } else {
                        self.filter.set("");
                        self.refresh_rows();
                    }
                }
                KeyCode::Down => self.action_move(1),
                KeyCode::Up => self.action_move(-1),
                KeyCode::Tab => self.focus = Focus::None,
                KeyCode::Enter => {
                    if self.filter.value().trim().is_empty() {
                        self.focus = Focus::None;
                        self.open_current();
                    } else {
                        self.run_search();
                    }
                }
                _ => {
                    if let Ed::Changed = self.filter.key(&k) {
                        self.refresh_rows();
                        self.list_scroll = 0;
                        if self.sel.as_ref().is_none_or(|s| !self.rows.iter().any(|r| &r.key == s))
                            && let Some(first) = self.rows.first().map(|r| r.key.clone())
                        {
                            self.sel = Some(first);
                        }
                        self.scroll_sel = true;
                    }
                }
            }
            return;
        }
        if self.narrow() && self.narrow_key(&k) {
            return;
        }
        if self.focus == Focus::Rail {
            match k.code {
                KeyCode::Down | KeyCode::Char('j') => return self.rail_move(1),
                KeyCode::Up | KeyCode::Char('k') => return self.rail_move(-1),
                KeyCode::Right | KeyCode::Char('l') | KeyCode::Enter | KeyCode::Esc => {
                    // → on a closed folder opens it first
                    if k.code == KeyCode::Right
                        && let Some(f) = self.view.strip_prefix("folder:").map(str::to_string)
                        && self.rail_entries().iter().any(|e| e.key.as_deref() == Some(&self.view) && e.fold == Some(false))
                    {
                        self.toggle_fold(&f);
                        return;
                    }
                    self.focus = Focus::None;
                    return;
                }
                KeyCode::Left | KeyCode::Char('h') => {
                    if let Some(f) = self.view.strip_prefix("folder:").map(str::to_string)
                        && self.open_folders.contains(&f)
                    {
                        self.toggle_fold(&f);
                    }
                    return;
                }
                _ => self.focus = Focus::None,
            }
        }
        match k.code {
            KeyCode::Char('q') if !ctrl => self.quit = true,
            KeyCode::Down | KeyCode::Char('j') if !alt => self.action_move(1),
            KeyCode::Up | KeyCode::Char('k') if !alt => self.action_move(-1),
            KeyCode::Left if alt => self.back(),
            KeyCode::Right if alt => self.forward(),
            KeyCode::Char('[') => self.back(),
            KeyCode::Char(']') => self.forward(),
            KeyCode::Left | KeyCode::Char('h') => {
                if self.geo().rail.width > 0 {
                    self.focus = Focus::Rail;
                    if self.rail_collapsed {
                        self.toggle_rail();
                    }
                }
            }
            KeyCode::Right | KeyCode::Char('l') => {}
            KeyCode::Enter => self.open_current(),
            KeyCode::Esc => self.escape(),
            KeyCode::Tab => self.next_link(1),
            KeyCode::BackTab => self.next_link(-1),
            KeyCode::Char('/') => self.focus_filter(),
            KeyCode::Char('f') if ctrl => self.focus_filter(),
            KeyCode::Char('n') => self.next_hit(1),
            KeyCode::Char('N') => self.next_hit(-1),
            KeyCode::Char(c @ '1'..='3') => self.run(&Act::Tab(c as usize - '1' as usize)),
            KeyCode::PageDown | KeyCode::Char(' ') => self.scroll_note(self.note_h.saturating_sub(2).max(1) as i64),
            KeyCode::PageUp => self.scroll_note(-(self.note_h.saturating_sub(2).max(1) as i64)),
            KeyCode::Char('d') if ctrl => self.scroll_note((self.note_h / 2).max(1) as i64),
            KeyCode::Char('u') if ctrl => self.scroll_note(-((self.note_h / 2).max(1) as i64)),
            KeyCode::Char('J') => self.scroll_note(1),
            KeyCode::Char('K') => self.scroll_note(-1),
            KeyCode::Home => self.note_scroll = 0,
            KeyCode::End => self.note_scroll = usize::MAX / 2,
            KeyCode::Char('e') => self.edit(None),
            KeyCode::Char('o') => self.run(&Act::Obsidian),
            KeyCode::Char('y') => self.run(&Act::CopyLink),
            KeyCode::Char('Y') => self.run(&Act::CopyPath),
            KeyCode::Char('s') => self.run(&Act::Sort),
            KeyCode::Char('b') => self.toggle_rail(),
            KeyCode::Char('{') => self.set_list_width(-4),
            KeyCode::Char('}') => self.set_list_width(4),
            KeyCode::Char('r') if !ctrl => {
                self.rescan_now();
                self.toast("", "Read the vault again.", crate::app::Sev::Info);
            }
            KeyCode::Char('?') => self.help(),
            _ => {}
        }
    }

    /// Keys that mean something different when a narrow window shows one
    /// pane. Returns true if the key was used.
    fn narrow_key(&mut self, k: &KeyEvent) -> bool {
        if k.modifiers.contains(KeyModifiers::ALT) {
            return false;
        }
        match self.shown() {
            Stage::Rail => match k.code {
                KeyCode::Down | KeyCode::Char('j') => self.rail_move(1),
                KeyCode::Up | KeyCode::Char('k') => self.rail_move(-1),
                KeyCode::Enter | KeyCode::Right | KeyCode::Char('l') => {
                    // → on a closed folder opens it first; enter always goes in
                    let closed = self.rail_entries().iter().any(|e| e.key.as_deref() == Some(&self.view) && e.fold == Some(false));
                    match self.view.strip_prefix("folder:").map(str::to_string) {
                        Some(f) if closed && k.code != KeyCode::Enter => self.toggle_fold(&f),
                        _ => self.stage = Stage::List,
                    }
                }
                KeyCode::Left | KeyCode::Char('h') => {
                    if let Some(f) = self.view.strip_prefix("folder:").map(str::to_string)
                        && self.open_folders.contains(&f)
                    {
                        self.toggle_fold(&f);
                    }
                }
                KeyCode::Esc => {}
                _ => return false,
            },
            Stage::List => match k.code {
                KeyCode::Left | KeyCode::Char('h') => self.stage_back(),
                KeyCode::Right | KeyCode::Char('l') => self.open_current(),
                _ => return false,
            },
            Stage::Note => match k.code {
                KeyCode::Down | KeyCode::Char('j') => self.scroll_note(1),
                KeyCode::Up | KeyCode::Char('k') => self.scroll_note(-1),
                KeyCode::Left | KeyCode::Char('h') => self.stage_back(),
                KeyCode::Right | KeyCode::Char('l') => {}
                _ => return false,
            },
        }
        true
    }

    fn menu_key(&mut self, k: KeyEvent) {
        let Some(m) = self.modals.last_mut() else { return };
        let enabled = |opts: &Vec<Option<(String, crate::rich::Line)>>, i: usize| {
            opts.get(i).is_some_and(|o| o.as_ref().is_some_and(|(k, _)| k != "-none"))
        };
        let n = m.opts.len();
        let step = |from: Option<usize>, d: i64, opts: &Vec<Option<(String, crate::rich::Line)>>| -> Option<usize> {
            if n == 0 {
                return None;
            }
            let mut i = match from {
                None => {
                    if d > 0 {
                        n - 1
                    } else {
                        0
                    }
                }
                Some(i) => i,
            };
            for _ in 0..n {
                i = ((i as i64 + d).rem_euclid(n as i64)) as usize;
                if enabled(opts, i) {
                    return Some(i);
                }
            }
            from
        };
        match k.code {
            KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('?') => {
                self.modals.pop();
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if m.opts.iter().all(|o| o.as_ref().is_none_or(|(k, _)| k == "-none")) {
                    m.top += 1;
                } else {
                    m.hi = step(m.hi, 1, &m.opts);
                }
            }
            KeyCode::Up | KeyCode::Char('k') => {
                if m.opts.iter().all(|o| o.as_ref().is_none_or(|(k, _)| k == "-none")) {
                    m.top = m.top.saturating_sub(1);
                } else {
                    m.hi = step(m.hi, -1, &m.opts);
                }
            }
            KeyCode::Enter => {
                if let Some(i) = m.hi
                    && enabled(&m.opts, i)
                {
                    let key = m.opts[i].as_ref().map(|o| o.0.clone());
                    self.menu_pick(key);
                }
            }
            _ => {}
        }
    }

    pub fn on_paste(&mut self, s: &str) {
        self.dirty = true;
        if self.focus == Focus::Filter {
            self.filter.insert(s.lines().next().unwrap_or(""));
            self.refresh_rows();
        }
    }

    pub fn on_mouse(&mut self, m: MouseEvent) {
        let (x, y) = (m.column, m.row);
        if self.dragging {
            match m.kind {
                MouseEventKind::Drag(_) => self.drag_to(x),
                MouseEventKind::Up(_) => {
                    self.dragging = false;
                    self.dirty = true;
                    self.save_state();
                }
                _ => {}
            }
            self.mouse = Some((x, y));
            return;
        }
        // press on the edge between the list and the note (either border) to drag it
        if m.kind == MouseEventKind::Down(MouseButton::Left)
            && self.modals.is_empty()
            && let Some(dx) = self.divider_x()
            && (x == dx || x == dx + 1)
            && y > 0
            && y + 2 < self.size.1
        {
            self.dragging = true;
            self.mouse = Some((x, y));
            self.dirty = true;
            return;
        }
        match m.kind {
            MouseEventKind::Moved | MouseEventKind::Drag(_) => {
                if self.mouse != Some((x, y)) {
                    self.mouse = Some((x, y));
                    self.mouse_at = Some(std::time::Instant::now());
                    self.dirty = true;
                }
            }
            MouseEventKind::ScrollDown => self.wheel(x, y, 3),
            MouseEventKind::ScrollUp => self.wheel(x, y, -3),
            MouseEventKind::Down(b) => {
                self.mouse = Some((x, y));
                self.click(x, y, b);
            }
            _ => {}
        }
    }

    /// The wheel scrolls whatever is under the mouse, and never moves focus.
    pub fn wheel(&mut self, x: u16, y: u16, d: i64) {
        self.dirty = true;
        let target = self
            .hits
            .iter()
            .rev()
            .find(|h| h.r.contains((x, y).into()) && matches!(h.k, HitKind::Scroll(_) | HitKind::Backdrop))
            .map(|h| h.k.clone());
        let bump = |v: &mut usize| *v = (*v as i64 + d).max(0) as usize;
        match target {
            Some(HitKind::Scroll(Scroll::Menu)) => {
                if let Some(m) = self.modals.last_mut() {
                    bump(&mut m.top);
                }
            }
            Some(HitKind::Scroll(s)) if self.modals.is_empty() => match s {
                Scroll::Rail => bump(&mut self.rail_scroll),
                Scroll::List => bump(&mut self.list_scroll),
                Scroll::Note => bump(&mut self.note_scroll),
                Scroll::Menu => {}
            },
            _ => {}
        }
    }

    pub fn click(&mut self, x: u16, y: u16, b: MouseButton) {
        self.dirty = true;
        let right = b == MouseButton::Right;
        let Some(hit) =
            self.hits.iter().rev().find(|h| h.r.contains((x, y).into()) && !matches!(h.k, HitKind::Scroll(_))).cloned()
        else {
            return;
        };
        // a click never leaves the arrow keys anywhere but the list
        if !matches!(hit.k, HitKind::FilterBox) {
            self.focus = Focus::None;
        }
        match hit.k {
            HitKind::Backdrop => {
                self.modals.pop();
            }
            HitKind::Blur => {}
            HitKind::FilterBox => self.focus = Focus::Filter,
            HitKind::Act(Act::MenuPick(i)) => {
                let key = self.modals.last().and_then(|m| m.opts.get(i).cloned().flatten()).map(|o| o.0);
                if key.is_some() {
                    self.menu_pick(key);
                }
            }
            HitKind::Act(Act::Toast(id)) => self.toasts.retain(|t| t.id != id),
            HitKind::Act(a) => {
                self.link_sel = None;
                self.run(&a);
            }
            HitKind::Rail(k, arrow) => {
                let f = k.strip_prefix("folder:").map(str::to_string);
                match (arrow, f) {
                    (Some(ax), Some(f)) if x >= ax && x < ax + 2 => self.toggle_fold(&f),
                    _ => self.set_view(&k),
                }
            }
            HitKind::Row(key) => {
                self.link_sel = None;
                self.sel = Some(key.clone());
                self.open_current();
                if right {
                    self.stage = Stage::List;
                    self.row_menu(&key, x as i32, y as i32);
                }
            }
            HitKind::Scroll(_) => {}
        }
    }
}
