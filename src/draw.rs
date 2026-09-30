//! Painting the frame: the sidebar, the note list, the note, the key bar,
//! then menus and toasts on top. Thin rounded frames, colour only where it
//! means something. Every clickable span is recorded while painting, so a
//! click can find what it landed on.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};

use crate::app::{App, Focus, Sev, Stage};
use crate::rich::{Act, Line, Seg, line_width, plain, sp, wrap};
use crate::util::{ago, cell_len, char_w, crop, fit, rjust};

#[derive(Clone, Debug)]
pub enum HitKind {
    Act(Act),
    Row(String),
    /// a sidebar entry: its key, and the x of its fold arrow if it has one
    Rail(String, Option<u16>),
    FilterBox,
    Scroll(Scroll),
    /// a graph's empty space: the wheel zooms, a drag pans
    Graph,
    /// clicking here takes focus away from the filter and the sidebar
    Blur,
    Backdrop,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Scroll {
    Rail,
    List,
    Note,
    Ask,
    Menu,
}

#[derive(Clone, Debug)]
pub struct Hit {
    pub r: Rect,
    pub k: HitKind,
}

pub struct Geo {
    pub rail: Rect,
    pub list: Option<Rect>,
    pub note: Option<Rect>,
    /// the whole vault's graph, in place of the list and the note
    pub graph: Option<Rect>,
    pub keybar: Rect,
}

fn inner(r: Rect) -> Rect {
    Rect::new(r.x + 1, r.y + 1, r.width.saturating_sub(2), r.height.saturating_sub(2))
}

impl App {
    pub fn geo(&self) -> Geo {
        let (w, h) = self.size;
        let main_h = h.saturating_sub(1);
        if self.narrow() {
            // one pane at a time, the whole width
            let full = Rect::new(0, 0, w, main_h);
            let none = Rect::new(0, 0, 0, main_h);
            let (rail, list, note, graph) = match self.shown() {
                Stage::Rail => (full, None, None, None),
                Stage::List if self.view == "graph" => (none, None, None, Some(full)),
                Stage::List => (none, Some(full), None, None),
                Stage::Note => (none, None, Some(full), None),
            };
            return Geo { rail, list, note, graph, keybar: Rect::new(0, main_h, w, 1) };
        }
        let rail_w = if self.rail_collapsed { 6 } else { 26 };
        let rest = w.saturating_sub(rail_w);
        if self.view == "graph" {
            let graph = Some(Rect::new(rail_w, 0, rest, main_h));
            return Geo { rail: Rect::new(0, 0, rail_w, main_h), list: None, note: None, graph, keybar: Rect::new(0, main_h, w, 1) };
        }
        let (list, note) = {
            let lw = self.list_w.clamp(28, rest.saturating_sub(40).max(28));
            (Some(Rect::new(rail_w, 0, lw, main_h)), Some(Rect::new(rail_w + lw, 0, rest - lw, main_h)))
        };
        Geo { rail: Rect::new(0, 0, rail_w, main_h), list, note, graph: None, keybar: Rect::new(0, main_h, w, 1) }
    }

    /// The rows area of the list.
    pub fn rows_area(&self) -> Rect {
        let Some(l) = self.geo().list else { return Rect::default() };
        let c = inner(l);
        Rect::new(c.x, c.y + 2, c.width, c.height.saturating_sub(2))
    }

    fn row_h(&self) -> usize {
        if self.view == "search" { 2 } else { 1 }
    }
}

pub struct P<'a> {
    pub buf: &'a mut Buffer,
    pub hits: Vec<Hit>,
    pub hover: Option<Act>,
    pub mouse: Option<(u16, u16)>,
    pub bg: Color,
    /// the link tab has picked: drawn highlighted
    pub chosen: Option<(u16, Act)>,
    pub sel_bg: Color,
}

impl P<'_> {
    fn area(&self) -> Rect {
        *self.buf.area()
    }

    pub fn fill(&mut self, r: Rect, bg: Color) {
        let r = r.intersection(self.area());
        for y in r.top()..r.bottom() {
            for x in r.left()..r.right() {
                if let Some(c) = self.buf.cell_mut((x, y)) {
                    c.set_symbol(" ");
                    c.set_style(Style::default().bg(bg));
                }
            }
        }
    }

    /// Put text at (x, y), no further than `maxx`. Returns the x after it.
    pub fn put(&mut self, x: u16, y: u16, text: &str, st: Style, maxx: u16) -> u16 {
        let area = self.area();
        if y >= area.bottom() {
            return x;
        }
        let mut cx = x;
        for ch in text.chars() {
            let w = char_w(ch) as u16;
            if w == 0 {
                continue;
            }
            if cx + w > maxx.min(area.right()) {
                break;
            }
            if let Some(c) = self.buf.cell_mut((cx, y)) {
                c.set_char(ch);
                c.set_style(st);
            }
            if w == 2
                && let Some(c) = self.buf.cell_mut((cx + 1, y))
            {
                c.set_symbol("");
            }
            cx += w;
        }
        cx
    }

    fn hovering(&self, r: Rect) -> bool {
        self.mouse.is_some_and(|(x, y)| x >= r.x && x < r.right() && y >= r.y && y < r.bottom())
    }

    fn style(&self, s: &Seg, bg: Color, y: u16) -> Style {
        let mut st = Style::default().bg(s.bg.unwrap_or(bg));
        if let Some(fg) = s.fg {
            st = st.fg(fg);
        }
        let mut m = Modifier::empty();
        if s.bold {
            m |= Modifier::BOLD;
        }
        if s.italic {
            m |= Modifier::ITALIC;
        }
        if s.strike {
            m |= Modifier::CROSSED_OUT;
        }
        if s.under || (s.act.is_some() && s.act == self.hover) {
            m |= Modifier::UNDERLINED;
        }
        if let Some((cy, a)) = &self.chosen
            && *cy == y
            && s.act.as_ref() == Some(a)
        {
            st = st.bg(self.sel_bg);
            m |= Modifier::UNDERLINED;
        }
        st.add_modifier(m)
    }

    /// One line of segments; clickable ones are recorded. Returns the x after it.
    pub fn line(&mut self, x: u16, y: u16, l: &[Seg], maxx: u16, bg: Color) -> u16 {
        let mut cx = x;
        for s in l {
            let st = self.style(s, bg, y);
            let nx = self.put(cx, y, &s.text, st, maxx);
            if let Some(a) = &s.act
                && nx > cx
            {
                self.hits.push(Hit { r: Rect::new(cx, y, nx - cx, 1), k: HitKind::Act(a.clone()) });
            }
            cx = nx;
        }
        cx
    }

    pub fn hit(&mut self, r: Rect, k: HitKind) {
        self.hits.push(Hit { r, k });
    }

    /// A rounded frame with a title on the top edge and a note on the bottom.
    pub fn frame(&mut self, r: Rect, col: Color, title: Option<(&str, Color, bool)>, sub: Option<(&str, Color)>) {
        if r.width < 2 || r.height < 2 {
            return;
        }
        let st = Style::default().fg(col).bg(self.bg);
        let (x0, x1, y0, y1) = (r.x, r.right() - 1, r.y, r.bottom() - 1);
        let hline = "─".repeat((r.width - 2) as usize);
        self.put(x0, y0, &format!("╭{hline}╮"), st, x1 + 1);
        self.put(x0, y1, &format!("╰{hline}╯"), st, x1 + 1);
        for y in y0 + 1..y1 {
            self.put(x0, y, "│", st, x0 + 1);
            self.put(x1, y, "│", st, x1 + 1);
        }
        if let Some((t, c, bold)) = title
            && !t.is_empty()
            && r.width > 6
        {
            let t = crop(t, (r.width - 6) as usize);
            let mut ts = Style::default().fg(c).bg(self.bg);
            if bold {
                ts = ts.add_modifier(Modifier::BOLD);
            }
            let x = self.put(x0 + 2, y0, " ", st, x1);
            let x = self.put(x, y0, &t, ts, x1);
            self.put(x, y0, " ", st, x1);
        }
        if let Some((t, c)) = sub
            && !t.is_empty()
            && r.width > 10
        {
            let t = crop(t, (r.width - 8) as usize);
            let start = x1 - 2 - cell_len(&t) as u16;
            let ss = Style::default().fg(c).bg(self.bg);
            self.put(start - 1, y1, " ", st, x1);
            let x = self.put(start, y1, &t, ss, x1);
            self.put(x, y1, " ", st, x1);
        }
    }

    /// A vertical scrollbar in column x over `h` rows.
    #[allow(clippy::too_many_arguments)]
    fn scrollbar(&mut self, x: u16, y: u16, h: u16, total: usize, top: usize, thumb: Color, track: Color) {
        if total <= h as usize || h == 0 {
            return;
        }
        let size = ((h as f64 * h as f64 / total as f64).round() as u16).clamp(1, h);
        let max_top = total - h as usize;
        let pos = ((top.min(max_top) as f64 / max_top as f64) * (h - size) as f64).round() as u16;
        for i in 0..h {
            let on = i >= pos && i < pos + size;
            if let Some(c) = self.buf.cell_mut((x, y + i)) {
                c.set_symbol(if on { "┃" } else { "│" });
                c.set_style(Style::default().fg(if on { thumb } else { track }).bg(self.bg));
            }
        }
    }
}

pub fn draw(app: &mut App, buf: &mut Buffer) {
    let area = *buf.area();
    app.size = (area.width, area.height);
    let hover = app.mouse.and_then(|(x, y)| {
        app.hits.iter().rev().find(|h| h.r.contains((x, y).into())).and_then(|h| match &h.k {
            HitKind::Act(a) => Some(a.clone()),
            _ => None,
        })
    });
    let bg = app.t.c("bg");
    let sel_bg = app.t.c("raise");
    let mut p = P { buf, hits: vec![], hover, mouse: app.mouse, bg, chosen: None, sel_bg };
    p.fill(area, bg);
    let g = app.geo();
    if g.rail.width > 0 {
        draw_rail(app, &mut p, g.rail);
    }
    if let Some(l) = g.list {
        draw_list(app, &mut p, l);
    }
    if let Some(n) = g.note {
        draw_note(app, &mut p, n);
    }
    if let Some(gr) = g.graph {
        let n = app.vault.notes.len();
        let (line, ink, faint) = (app.t.c("line"), app.t.c("ink"), app.t.c("faint"));
        let sub = format!("{n} note{} · wheel zooms · drag pans · 0 resets", if n == 1 { "" } else { "s" });
        let title = if app.narrow() { "‹ Graph" } else { "Graph" };
        p.frame(gr, line, Some((title, ink, true)), Some((&sub, faint)));
        if app.narrow() {
            p.hit(Rect::new(gr.x + 2, gr.y, 10, 1), HitKind::Act(Act::StageBack));
        }
        draw_graph(app, &mut p, inner(gr), true);
    }
    let kb = app.keybar();
    let kb = wrap(&kb, g.keybar.width as usize).into_iter().next().unwrap_or_default();
    p.line(g.keybar.x + 1, g.keybar.y, &kb, g.keybar.right(), bg);
    if !app.modals.is_empty() {
        draw_menu(app, &mut p);
    }
    draw_toasts(app, &mut p);
    draw_tooltip(app, &mut p);
    app.hits = p.hits;
}

fn draw_rail(app: &mut App, p: &mut P, r: Rect) {
    let (line, faint, dim, ink, acc, panel, raise) = (
        app.t.c("line"),
        app.t.c("faint"),
        app.t.c("dim"),
        app.t.c("ink"),
        app.t.c("accent"),
        app.t.c("panel"),
        app.t.c("raise"),
    );
    let narrow = app.narrow();
    let col = if app.focus == Focus::Rail || narrow { faint } else { line };
    p.frame(r, col, None, None);
    let inn = inner(r);
    if inn.width == 0 {
        return;
    }
    p.hit(inn, HitKind::Blur);
    p.hit(inn, HitKind::Scroll(Scroll::Rail));
    let w = inn.width as usize;
    // the head: the vault's name and «, or » when folded
    let hr = Rect::new(inn.x, inn.y, inn.width, 1);
    let hbg = if p.hovering(hr) { panel } else { p.bg };
    p.fill(hr, hbg);
    if narrow {
        let name = crop(&app.vault.name, w.saturating_sub(2));
        p.put(inn.x, inn.y, &format!(" {name}"), Style::default().fg(dim).bg(p.bg).add_modifier(Modifier::BOLD), inn.right());
    } else if app.rail_collapsed {
        p.put(inn.x, inn.y, &fit(" »", w), Style::default().fg(faint).bg(hbg), inn.right());
    } else {
        let name = crop(&app.vault.name, w.saturating_sub(4));
        let x = p.put(inn.x, inn.y, &format!(" {name}"), Style::default().fg(dim).bg(hbg).add_modifier(Modifier::BOLD), inn.right());
        let pad = w.saturating_sub(cell_len(&name) + 2);
        p.put(x, inn.y, &(" ".repeat(pad) + "«"), Style::default().fg(faint).bg(hbg), inn.right());
    }
    if !narrow {
        p.hit(hr, HitKind::Act(Act::ToggleRail));
    }
    let entries = app.rail_entries();
    let h = inn.height.saturating_sub(1) as usize;
    // keep the current entry in view
    if let Some(i) = entries.iter().position(|e| e.key.as_deref() == Some(app.view.as_str()))
        && (app.focus == Focus::Rail || app.scroll_sel || narrow)
    {
        if i < app.rail_scroll {
            app.rail_scroll = i;
        } else if i >= app.rail_scroll + h {
            app.rail_scroll = i + 1 - h;
        }
    }
    app.rail_scroll = app.rail_scroll.min(entries.len().saturating_sub(h));
    let mut tip = None;
    for (i, e) in entries.iter().enumerate().skip(app.rail_scroll).take(h) {
        let y = inn.y + 1 + (i - app.rail_scroll) as u16;
        let rr = Rect::new(inn.x, y, inn.width, 1);
        let Some(k) = &e.key else {
            if !app.rail_collapsed {
                p.put(inn.x, y, &format!(" {}", e.label.to_uppercase()), Style::default().fg(faint).bg(p.bg), inn.right());
            } else {
                p.put(inn.x, y, &" ".repeat(w), Style::default().bg(p.bg), inn.right());
            }
            continue;
        };
        let on = app.view == *k;
        let cursor = on && (app.focus == Focus::Rail || narrow);
        let rbg = if cursor {
            raise
        } else if p.hovering(rr) {
            panel
        } else {
            p.bg
        };
        p.fill(rr, rbg);
        let x = p.put(inn.x, y, if on { "▌" } else { " " }, Style::default().fg(acc).bg(rbg), inn.right());
        let colr = if on { ink } else { dim };
        let mut st = Style::default().fg(colr).bg(rbg);
        if on {
            st = st.add_modifier(Modifier::BOLD);
        }
        if app.rail_collapsed && !narrow {
            p.put(x, y, &fit(&e.short, w - 1), st, inn.right());
            p.hit(rr, HitKind::Rail(k.clone(), None));
        } else {
            let indent = e.depth * 2;
            let x = p.put(x, y, &" ".repeat(indent), st, inn.right());
            let arrow_x = x;
            let arrow = match e.fold {
                Some(true) => "▾ ",
                Some(false) => "▸ ",
                None if k.starts_with("folder:") => "  ",
                None => "",
            };
            let x = p.put(x, y, arrow, Style::default().fg(faint).bg(rbg), inn.right());
            let used = 1 + indent + cell_len(arrow);
            let lw = w.saturating_sub(used + e.count.chars().count() + 1);
            let x = p.put(x, y, &fit(&e.label, lw), st, inn.right());
            p.put(x, y, &format!(" {}", e.count), Style::default().fg(faint).bg(rbg), inn.right());
            p.hit(rr, HitKind::Rail(k.clone(), e.fold.map(|_| arrow_x)));
        }
        let rested = app.mouse_at.is_some_and(|t| t.elapsed() >= std::time::Duration::from_millis(500));
        if app.rail_collapsed && rested && p.hovering(rr) {
            tip = Some(e.label.clone());
        }
    }
    if entries.len() > h {
        p.scrollbar(r.right() - 1, inn.y + 1, h as u16, entries.len(), app.rail_scroll, faint, line);
    }
    app.tooltip = tip;
}

fn draw_tooltip(app: &mut App, p: &mut P) {
    let (Some(t), Some((mx, my))) = (app.tooltip.clone(), app.mouse) else { return };
    if !app.modals.is_empty() {
        return;
    }
    let (w, h) = app.size;
    let tw = (cell_len(&t) as u16 + 4).min(w);
    let x = (mx + 1).min(w.saturating_sub(tw));
    let y = if my + 4 <= h { my + 1 } else { my.saturating_sub(3) };
    let r = Rect::new(x, y, tw, 3);
    let (panel, line, ink) = (app.t.c("panel"), app.t.c("line"), app.t.c("ink"));
    p.fill(r, panel);
    let old = p.bg;
    p.bg = panel;
    p.frame(r, line, None, None);
    p.put(x + 2, y + 1, &t, Style::default().fg(ink).bg(panel), x + tw - 1);
    p.bg = old;
}

fn draw_list(app: &mut App, p: &mut P, r: Rect) {
    let (ink, faint, dim, line, acc) = (app.t.c("ink"), app.t.c("faint"), app.t.c("dim"), app.t.c("line"), app.t.c("accent"));
    let focused = app.focus == Focus::Filter;
    let n = app.rows.len();
    let sub = format!(
        "{} note{}{}",
        n,
        if n == 1 { "" } else { "s" },
        match app.view.as_str() {
            "search" => " · best first".to_string(),
            "recent" => " · newest first".to_string(),
            _ if app.sort_mod => " · newest first".to_string(),
            _ => " · a to z".to_string(),
        }
    );
    let title = if app.narrow() { format!("‹ {}", app.view_label()) } else { app.view_label() };
    p.frame(r, if focused { faint } else { line }, Some((&title, ink, true)), Some((&sub, faint)));
    if app.narrow() {
        let tw = (crate::util::cell_len(&title) as u16 + 2).min(r.width.saturating_sub(4));
        p.hit(Rect::new(r.x + 2, r.y, tw, 1), HitKind::Act(Act::StageBack));
    }
    let inn = inner(r);
    if inn.width < 4 || inn.height < 3 {
        return;
    }
    // the find bar: ⌕, the box, the sort switch
    let fx = inn.x + 1;
    p.put(fx, inn.y, "⌕", Style::default().fg(if focused { acc } else { faint }).bg(p.bg), fx + 2);
    let sort_label = if app.sort_mod { "↕ date" } else { "↕ name" };
    let show_sort = app.view != "search" && app.view != "recent";
    let sw = if show_sort { cell_len(sort_label) as u16 + 1 } else { 0 };
    let qx = fx + 2;
    let qw = inn.width.saturating_sub(4 + sw);
    let ph = if app.view == "search" { "filter · enter to search again" } else { "filter · enter to search all" };
    input_box(p, &mut app.filter, qx, inn.y, qw, ph, focused, ink, faint, acc);
    p.hit(Rect::new(fx, inn.y, qw + 2, 1), HitKind::FilterBox);
    if show_sort {
        p.line(qx + qw + 1, inn.y, &[sp(sort_label, faint).on(Act::Sort)], inn.right(), p.bg);
    }
    let soft = app.t.c("line-soft");
    p.put(inn.x, inn.y + 1, &"─".repeat(inn.width as usize), Style::default().fg(soft).bg(p.bg), inn.right());

    // the right edge can be dragged: show it when the mouse is on it
    let on_edge = app.dragging
        || (app.modals.is_empty()
            && app.divider_x().is_some_and(|dx| p.mouse.is_some_and(|(mx, my)| (mx == dx || mx == dx + 1) && my > r.y && my + 1 < r.bottom())));
    if on_edge {
        for y in r.y + 1..r.bottom() - 1 {
            p.put(r.right() - 1, y, "┃", Style::default().fg(acc).bg(p.bg), r.right());
        }
    }

    let a = app.rows_area();
    let rh = app.row_h();
    let per = (a.height as usize / rh).max(1);
    let total = app.rows.len();
    let scroll = total > per;
    let w = (a.width as usize).saturating_sub(if scroll { 1 } else { 0 });
    if app.scroll_sel {
        app.scroll_sel = false;
        if let Some(i) = app.sel.as_ref().and_then(|s| app.rows.iter().position(|r| &r.key == s)) {
            if i < app.list_scroll {
                app.list_scroll = i;
            } else if i >= app.list_scroll + per {
                app.list_scroll = i + 1 - per;
            }
        }
    }
    app.list_scroll = app.list_scroll.min(total.saturating_sub(per));
    p.hit(a, HitKind::Blur);
    p.hit(a, HitKind::Scroll(Scroll::List));
    if total == 0 {
        let msg = if app.vault.notes.is_empty() {
            "No notes in this vault."
        } else if !app.filter.value().is_empty() {
            "No titles match. Press enter to search inside the notes."
        } else {
            "Nothing here."
        };
        for (i, l) in wrap(&[sp(msg, dim)], w.saturating_sub(4)).iter().enumerate() {
            p.line(a.x + 2, a.y + 1 + i as u16, l, a.right(), p.bg);
        }
        return;
    }
    let (panel, raise) = (app.t.c("panel"), app.t.c("raise"));
    let terms = crate::vault::terms(&app.query);
    let rows = app.rows.clone();
    for (i, row) in rows.iter().enumerate().skip(app.list_scroll).take(per) {
        let y = a.y + ((i - app.list_scroll) * rh) as u16;
        let rr = Rect::new(a.x, y, w as u16, rh as u16);
        let Some(note) = app.vault.note(&row.key) else { continue };
        let sel = app.sel.as_deref() == Some(row.key.as_str());
        let rbg = if sel {
            raise
        } else if p.hovering(rr) {
            panel
        } else {
            p.bg
        };
        p.fill(rr, rbg);
        let when = ago(note.mtime);
        let right = rjust(&when, 4);
        let avail = w.saturating_sub(3 + cell_len(&right));
        let name_w = cell_len(&note.name).min(avail);
        let mut st = Style::default().fg(if sel { ink } else { dim }).bg(rbg);
        if sel {
            st = st.add_modifier(Modifier::BOLD);
        }
        let x = p.put(a.x, y, if sel { "▌" } else { " " }, Style::default().fg(acc).bg(rbg), a.right());
        let x = p.put(x, y, &fit(&note.name, name_w), st, a.x + w as u16);
        let folder_w = avail.saturating_sub(name_w + 2);
        let show_folder = !note.folder.is_empty() && !app.view.starts_with("folder:") && folder_w >= 6;
        if show_folder {
            p.put(x + 2, y, &fit(&note.folder, folder_w), Style::default().fg(faint).bg(rbg), a.x + w as u16);
        }
        p.put(a.x + (w - cell_len(&right) - 1) as u16, y, &right, Style::default().fg(faint).bg(rbg), a.x + w as u16);
        if let Some(snip) = &row.snippet
            && rh == 2
        {
            let text = crop(snip, w.saturating_sub(4));
            let segs = vec![sp(text, faint)];
            let spans = crate::vault::find_ci(&crate::md::line_text(&segs), &terms);
            let segs = crate::md::mark(&segs, &spans, rbg, acc);
            let segs: Vec<Seg> = segs.into_iter().map(|mut s| {
                if s.fg == Some(acc) {
                    s.bold = true;
                    s.bg = None;
                }
                s
            }).collect();
            p.line(a.x + 3, y + 1, &segs, a.x + w as u16, rbg);
        }
        p.hit(rr, HitKind::Row(row.key.clone()));
    }
    if scroll {
        p.scrollbar(a.x + w as u16, a.y, a.height, total * rh, app.list_scroll * rh, faint, line);
    }
}

#[allow(clippy::too_many_arguments)]
fn input_box(
    p: &mut P,
    ed: &mut crate::input::LineEdit,
    x: u16,
    y: u16,
    w: u16,
    placeholder: &str,
    focused: bool,
    ink: Color,
    faint: Color,
    acc: Color,
) {
    if w == 0 {
        return;
    }
    let bg = p.bg;
    let maxx = x + w;
    // the cursor is drawn reversed, so it shows on any background
    let cur = Style::default().add_modifier(Modifier::REVERSED);
    if ed.chars.is_empty() {
        p.put(x, y, placeholder, Style::default().fg(faint).bg(bg), maxx);
        if focused {
            let first = placeholder.chars().next().unwrap_or(' ').to_string();
            p.put(x, y, &first, cur.fg(acc), maxx);
        }
        return;
    }
    let (text, cx) = ed.view(w as usize);
    p.put(x, y, &text, Style::default().fg(ink).bg(bg), maxx);
    if focused {
        let mut acc_w = 0usize;
        let under = text
            .chars()
            .find(|c| {
                let here = acc_w == cx;
                acc_w += char_w(*c);
                here
            })
            .map(|c| c.to_string())
            .unwrap_or(" ".into());
        p.put(x + cx as u16, y, &under, cur.fg(ink), maxx);
    }
}

fn draw_note(app: &mut App, p: &mut P, r: Rect) {
    if app.editor.is_some() {
        draw_editor(app, p, r);
        return;
    }
    if app.asking {
        draw_ask(app, p, r);
        return;
    }
    let (ink, faint, dim, line, acc) = (app.t.c("ink"), app.t.c("faint"), app.t.c("dim"), app.t.c("line"), app.t.c("accent"));
    let Some(key) = app.note.clone() else {
        p.frame(r, line, None, None);
        let msg = if app.vault.notes.is_empty() { "This vault has no notes yet." } else { "Pick a note on the left." };
        p.put(r.x + 3, r.y + 2, msg, Style::default().fg(dim).bg(p.bg), r.right() - 2);
        return;
    };
    let Some(note) = app.vault.note(&key) else { return };
    let (name, folder, mtime) = (note.name.clone(), note.folder.clone(), note.mtime);
    let sub = if app.narrow() { "esc back to the notes · e edit" } else { "e edit · E $EDITOR · o obsidian" };
    p.frame(r, line, Some((&name, ink, true)), Some((sub, faint)));
    let inn = Rect::new(r.x + 2, r.y + 1, r.width.saturating_sub(4), r.height.saturating_sub(2));
    if inn.width < 4 || inn.height < 4 {
        return;
    }
    // where it lives, and when it last changed
    let mut head: Line = vec![];
    if app.narrow() {
        head.push(sp("‹ notes", dim).on(Act::StageBack));
        head.push(sp("  ·  ", faint));
    }
    if folder.is_empty() {
        head.push(sp(app.vault.name.clone(), faint).on(Act::Rail("all".into())));
    } else {
        let mut acc_path = String::new();
        for (i, part) in folder.split('/').enumerate() {
            if i > 0 {
                head.push(sp(" › ", faint));
                acc_path.push('/');
            }
            acc_path.push_str(part);
            head.push(sp(part.to_string(), faint).on(Act::Rail(format!("folder:{acc_path}"))));
        }
    }
    head.push(sp(format!("  ·  edited {} ago", ago(mtime)).replace("edited now ago", "edited just now"), faint));
    if app.hist_pos > 0 {
        head.push(plain("  "));
        head.push(sp("‹ back", faint).on(Act::Back));
    }
    if app.hist_pos + 1 < app.history.len() {
        head.push(plain("  "));
        head.push(sp("forward ›", faint).on(Act::Forward));
    }
    let hl = wrap(&head, inn.width as usize).into_iter().next().unwrap_or_default();
    p.line(inn.x, inn.y, &hl, inn.right(), p.bg);
    // tabs
    let back_n = app.vault.backlinks.get(&key).map(|v| v.len()).unwrap_or(0);
    let bw = app.body_width();
    let heads_n = app.rendered(bw).map(|r| r.headings.len()).unwrap_or(0);
    let tabs = [
        ("Note".to_string(), 0usize),
        (format!("Links {back_n}"), 1),
        (format!("Outline {heads_n}"), 2),
        ("Graph".to_string(), 3),
    ];
    let ty = inn.y + 2;
    let mut x = inn.x;
    let soft = app.t.c("line-soft");
    p.put(inn.x, ty + 1, &"─".repeat(inn.width as usize), Style::default().fg(soft).bg(p.bg), inn.right());
    for (label, i) in tabs {
        let on = app.tab == i;
        let seg = sp(format!(" {label} "), if on { ink } else { dim }).b_if(on).on(Act::Tab(i));
        let nx = p.line(x, ty, &[seg], inn.right(), p.bg);
        if on {
            p.put(x, ty + 1, &"━".repeat((nx - x) as usize), Style::default().fg(acc).bg(p.bg), inn.right());
        }
        x = nx + 1;
    }
    if !app.terms.is_empty() && app.tab == 0 {
        let hint = vec![sp("n", ink), sp(" next match ", faint), sp("N", ink), sp(" back", faint)];
        let hw = line_width(&hint) as u16;
        if x + hw + 2 < inn.right() {
            p.line(inn.right() - hw, ty, &hint, inn.right(), p.bg);
        }
    }
    // the body
    let body = Rect::new(inn.x, ty + 3, inn.width, inn.bottom().saturating_sub(ty + 3));
    if app.tab == 3 {
        let hint = format!("depth {} · + more · − fewer · 0 reset", app.graph_depth);
        let hw = crate::util::cell_len(&hint) as u16;
        if body.height > 3 {
            draw_graph(app, p, Rect::new(body.x, body.y, body.width, body.height - 1), false);
            let hy = body.bottom() - 1;
            let segs = vec![
                sp(format!("depth {}", app.graph_depth), faint),
                sp(" · ", faint),
                sp("+ more", dim).on(Act::Depth(1)),
                sp(" · ", faint),
                sp("− fewer", dim).on(Act::Depth(-1)),
                sp(" · ", faint),
                sp("0 reset", dim).on(Act::GraphReset),
            ];
            p.line(body.right().saturating_sub(hw), hy, &segs, body.right(), p.bg);
        }
        return;
    }
    app.note_h = body.height as usize;
    let w = bw;
    app.apply_scroll_to(w);
    let (lines, links, _) = app.body(w);
    let h = body.height as usize;
    app.note_scroll = app.note_scroll.min(lines.len().saturating_sub(h));
    p.hit(body, HitKind::Blur);
    p.hit(body, HitKind::Scroll(Scroll::Note));
    p.chosen = app.link_sel.and_then(|i| links.get(i)).map(|(l, a)| {
        let y = body.y as i64 + *l as i64 - app.note_scroll as i64;
        (y.max(0) as u16, a.clone())
    });
    for (i, l) in lines.iter().enumerate().skip(app.note_scroll).take(h) {
        let ly = body.y + (i - app.note_scroll) as u16;
        p.line(body.x, ly, l, body.x + w as u16, p.bg);
    }
    p.chosen = None;
    if lines.len() > h {
        p.scrollbar(r.right() - 2, body.y, body.height, lines.len(), app.note_scroll, faint, line);
    }
}

/// A graph in `area`: the open note's (`whole` false) or the vault's.
/// Lines are braille on a Canvas; dots and labels are drawn over them, and
/// every dot and label clicks through to its note.
fn draw_graph(app: &mut App, p: &mut P, area: Rect, whole: bool) {
    use crate::graph::{ASPECT, Xform};
    use ratatui::symbols::Marker;
    use ratatui::widgets::Widget;
    use ratatui::widgets::canvas::{Canvas, Line as CLine};
    if area.width < 6 || area.height < 3 {
        return;
    }
    let g = if whole {
        app.vault_graph()
    } else {
        match app.local_graph() {
            Some(g) => g,
            None => return,
        }
    };
    let view = if whole { app.gview_vault } else { app.gview_local };
    let (w, h) = (area.width as f64, area.height as f64);
    // fit it all in, leaving room for labels; a local graph keeps its note
    // in the middle
    let (x0, y0, x1, y1) = g.bounds();
    let (fcx, fcy, bw, bh) = if whole {
        ((x0 + x1) / 2.0, (y0 + y1) / 2.0, (x1 - x0).max(20.0), (y1 - y0).max(20.0))
    } else {
        (0.0, 0.0, 2.0 * x0.abs().max(x1.abs()).max(10.0), 2.0 * y0.abs().max(y1.abs()).max(10.0))
    };
    let fit = ((w - 12.0).max(4.0) / bw).min((h - 2.0).max(2.0) / (bh * ASPECT));
    let xf = Xform {
        mid_x: area.x as f64 + w / 2.0,
        mid_y: area.y as f64 + h / 2.0,
        scale: fit * view.zoom,
        cx: fcx + view.pan_x,
        cy: fcy + view.pan_y,
    };
    app.graph_x = xf;
    app.graph_area = area;
    p.hit(area, HitKind::Graph);

    let cells: Vec<(f64, f64)> = g.nodes.iter().map(|n| xf.to_screen(n.x, n.y)).map(|(x, y)| (x.floor(), y.floor())).collect();
    let inside = |(x, y): (f64, f64)| x >= area.x as f64 && x < area.right() as f64 && y >= area.y as f64 && y < area.bottom() as f64;
    let hover = match &p.hover {
        Some(Act::GraphNode(id)) => g.find(id),
        _ => None,
    };
    let picked = app.graph_sel.as_ref().and_then(|s| g.find(s));
    let focus: Vec<usize> = hover.into_iter().chain(picked).collect();
    let near = |i: usize| focus.iter().any(|&f| f == i || g.edges.iter().any(|&(a, b)| (a == f && b == i) || (b == f && a == i)));

    // folder colours from the theme
    let mut folders: Vec<&str> = g.nodes.iter().filter(|n| !n.folder.is_empty()).map(|n| n.folder.as_str()).collect();
    folders.sort();
    folders.dedup();
    let pal = ["s2", "s3", "cyan", "teal", "amber", "s5", "flame", "sea", "s4", "s1"];
    let colour = |n: &crate::graph::GNode| -> Color {
        match folders.iter().position(|f| *f == n.folder) {
            Some(i) if !n.folder.is_empty() => app.t.c(pal[i % pal.len()]),
            _ => app.t.c("dim"),
        }
    };
    let (line_c, acc, ink, faint, dim, raise) =
        (app.t.c("line"), app.t.c("accent"), app.t.c("ink"), app.t.c("faint"), app.t.c("dim"), app.t.c("raise"));

    // lines, then the ones touching the dot in focus on top
    let top = area.y as f64 + h;
    let centre_of = |i: usize| (cells[i].0 + 0.5, top - (cells[i].1 + 0.5));
    let lit: Vec<(usize, usize)> = g.edges.iter().copied().filter(|&(a, b)| focus.contains(&a) || focus.contains(&b)).collect();
    let canvas = Canvas::default()
        .marker(Marker::Braille)
        .background_color(p.bg)
        .x_bounds([area.x as f64, area.right() as f64])
        .y_bounds([0.0, h])
        .paint(|ctx| {
            for &(a, b) in &g.edges {
                let ((ax, ay), (bx, by)) = (centre_of(a), centre_of(b));
                ctx.draw(&CLine { x1: ax, y1: ay, x2: bx, y2: by, color: line_c });
            }
            ctx.layer();
            for &(a, b) in &lit {
                let ((ax, ay), (bx, by)) = (centre_of(a), centre_of(b));
                ctx.draw(&CLine { x1: ax, y1: ay, x2: bx, y2: by, color: acc });
            }
        });
    canvas.render(area, p.buf);

    // dots
    let mut taken = vec![false; area.width as usize * area.height as usize];
    let take = |x: u16, y: u16, taken: &mut Vec<bool>| {
        let i = (y - area.y) as usize * area.width as usize + (x - area.x) as usize;
        let was = taken[i];
        taken[i] = true;
        was
    };
    app.graph_pts.clear();
    let mut shown = vec![];
    for (i, n) in g.nodes.iter().enumerate() {
        if !inside(cells[i]) {
            continue;
        }
        let (x, y) = (cells[i].0 as u16, cells[i].1 as u16);
        let centre = g.centre == Some(i);
        let (glyph, col) = if centre {
            ("◉", acc)
        } else if n.missing {
            ("○", faint)
        } else {
            ("●", colour(n))
        };
        let mut st = Style::default().fg(col).bg(p.bg);
        if focus.contains(&i) {
            st = st.bg(raise).add_modifier(Modifier::BOLD);
        } else if !focus.is_empty() && !near(i) {
            st = st.add_modifier(Modifier::DIM);
        }
        p.put(x, y, glyph, st, area.right());
        take(x, y, &mut taken);
        p.hit(Rect::new(x, y, 1, 1), HitKind::Act(Act::GraphNode(n.id.clone())));
        app.graph_pts.push((n.id.clone(), x as f64, y as f64));
        shown.push(i);
    }
    // labels: the dot in focus, the note in the middle, then the best linked
    let crowded = shown.len() > 40;
    let mut order = shown.clone();
    order.sort_by(|&a, &b| {
        let rank = |i: usize| (!focus.contains(&i), g.centre != Some(i), std::cmp::Reverse(g.nodes[i].degree));
        rank(a).cmp(&rank(b)).then_with(|| g.nodes[a].label.cmp(&g.nodes[b].label))
    });
    let limit = if crowded { 120 } else { order.len() };
    for &i in order.iter().take(limit) {
        let n = &g.nodes[i];
        let (x, y) = (cells[i].0 as u16, cells[i].1 as u16);
        let text = crate::util::crop(&n.label, 24);
        let tw = cell_len(&text) as u16;
        let fits = |sx: u16, taken: &Vec<bool>| {
            sx >= area.x
                && sx + tw <= area.right()
                && (sx.saturating_sub(1).max(area.x)..(sx + tw + 1).min(area.right()))
                    .all(|cx| !taken[(y - area.y) as usize * area.width as usize + (cx - area.x) as usize] || (cx == x))
        };
        let spot = [x + 2, x.saturating_sub(tw + 1)].into_iter().find(|&sx| fits(sx, &taken));
        let Some(sx) = spot else { continue };
        let is_focus = focus.contains(&i);
        let mut st = Style::default().fg(if is_focus || g.centre == Some(i) {
            ink
        } else if n.missing || (crowded && !near(i)) {
            faint
        } else {
            dim
        });
        st = st.bg(p.bg);
        if is_focus {
            st = st.bg(raise).add_modifier(Modifier::BOLD);
        } else if g.centre == Some(i) {
            st = st.add_modifier(Modifier::BOLD);
        } else if !focus.is_empty() && !near(i) {
            st = st.add_modifier(Modifier::DIM);
        }
        p.put(sx, y, &text, st, area.right());
        for cx in sx..sx + tw {
            take(cx, y, &mut taken);
        }
        p.hit(Rect::new(sx, y, tw, 1), HitKind::Act(Act::GraphNode(n.id.clone())));
    }
    if g.nodes.len() <= 1 && !whole {
        let msg = "No links in or out yet.";
        p.put(area.x + 1, area.y, msg, Style::default().fg(dim).bg(p.bg), area.right());
    }
    if g.capped {
        let msg = format!("showing the first {} notes", crate::graph::LOCAL_MAX);
        p.put(area.x + 1, area.y, &msg, Style::default().fg(faint).bg(p.bg), area.right());
    }
}

/// The ask pane, in the note's place: the conversation, then the question box.
fn draw_ask(app: &mut App, p: &mut P, r: Rect) {
    let (ink, faint, dim, acc, line) = (app.t.c("ink"), app.t.c("faint"), app.t.c("dim"), app.t.c("accent"), app.t.c("line"));
    let soft = app.t.c("line-soft");
    let busy = app.chat.busy();
    let sub = if busy { "esc stop" } else { "enter ask · esc close" };
    p.frame(r, acc, Some(("Ask your notes", ink, true)), Some((sub, faint)));
    let inn = Rect::new(r.x + 2, r.y + 1, r.width.saturating_sub(4), r.height.saturating_sub(2));
    if inn.width < 8 || inn.height < 6 {
        return;
    }
    // the head: what answers (click to change), and a fresh start
    let mut head: Line = vec![];
    if app.narrow() {
        head.push(sp("‹ notes", dim).on(Act::StageBack));
        head.push(sp("  ·  ", faint));
    }
    match &app.ask_model {
        Some(m) => {
            head.push(sp("answering: ", faint));
            head.push(sp(format!("{} ▾", m.label), ink).on(Act::AskModel));
        }
        None => head.push(sp("pick what answers ▾", dim).on(Act::AskModel)),
    }
    if !app.chat.turns.is_empty() {
        head.push(sp("  ·  ", faint));
        head.push(sp("new chat", dim).on(Act::AskNew));
    }
    let hl = wrap(&head, inn.width as usize).into_iter().next().unwrap_or_default();
    p.line(inn.x, inn.y, &hl, inn.right(), p.bg);
    p.put(inn.x, inn.y + 1, &"─".repeat(inn.width as usize), Style::default().fg(soft).bg(p.bg), inn.right());
    let body = Rect::new(inn.x, inn.y + 2, inn.width, inn.height.saturating_sub(4));
    let w = body.width.saturating_sub(2) as usize;
    let lines = ask_lines(app, w);
    let h = body.height as usize;
    let max = lines.len().saturating_sub(h);
    let top = app.chat.scroll.unwrap_or(max).min(max);
    app.chat.lines = lines.len();
    app.chat.rows = h;
    p.hit(body, HitKind::Blur);
    p.hit(body, HitKind::Scroll(Scroll::Ask));
    for (i, l) in lines.iter().enumerate().skip(top).take(h) {
        p.line(body.x, body.y + (i - top) as u16, l, body.x + w as u16, p.bg);
    }
    if lines.len() > h {
        p.scrollbar(r.right() - 2, body.y, body.height, lines.len(), top, faint, line);
    }
    // the question box
    let by = inn.bottom() - 1;
    p.put(inn.x, by - 1, &"─".repeat(inn.width as usize), Style::default().fg(soft).bg(p.bg), inn.right());
    let x = p.put(inn.x, by, "› ", Style::default().fg(acc).bg(p.bg).add_modifier(Modifier::BOLD), inn.right());
    let ph = if busy { "answering…" } else { "ask about your notes" };
    input_box(p, &mut app.chat.input, x, by, inn.right().saturating_sub(x), ph, !busy, ink, faint, acc);
}

/// The conversation as lines `w` wide: each question, its answer (drawn as
/// Markdown, links clickable) and the notes it looked at.
fn ask_lines(app: &mut App, w: usize) -> Vec<Line> {
    let (ink, faint, dim, acc, flame) = (app.t.c("ink"), app.t.c("faint"), app.t.c("dim"), app.t.c("accent"), app.t.c("flame"));
    let mut out: Vec<Line> = vec![];
    let para = |out: &mut Vec<Line>, segs: Vec<Seg>| out.extend(wrap(&segs, w));
    if app.chat.turns.is_empty() {
        para(&mut out, vec![sp(
            "Ask a question about the notes in this vault. It finds the notes that match best and answers from them, naming the ones it used.",
            dim,
        )]);
        out.push(vec![]);
        para(&mut out, vec![sp("Try: “what did I decide about the budget?”, “summarise this note”, “where did I write about compost?”", faint)]);
        out.push(vec![]);
        match &app.ask_model {
            Some(m) if m.local() => para(&mut out, vec![sp("It runs on this computer with Ollama: nothing leaves it.", faint)]),
            Some(_) => para(&mut out, vec![sp("Your question and the notes that match it go to Claude, on your Claude plan.", faint)]),
            None => {
                para(&mut out, vec![sp("Nothing to answer with yet.", flame).b()]);
                let why = app.ask_why.clone().unwrap_or_default();
                para(&mut out, vec![sp(
                    format!("{why} Install Ollama (ollama.com) and run “ollama pull llama3.2”, or install Claude Code to use your Claude plan. Then press ctrl+o."),
                    dim,
                )]);
            }
        }
        return out;
    }
    let tg = app.theme_gen;
    let n = app.chat.turns.len();
    for i in 0..n {
        if i > 0 {
            out.push(vec![]);
        }
        let t = &app.chat.turns[i];
        // the question
        let q = wrap(&[sp(t.q.clone(), ink).b()], w.saturating_sub(2));
        for (j, l) in q.into_iter().enumerate() {
            let mut row = vec![if j == 0 { sp("› ", acc).b() } else { plain("  ") }];
            row.extend(l);
            out.push(row);
        }
        out.push(vec![]);
        // the answer so far
        let answer = t.answer();
        if answer.trim().is_empty() && t.err.is_none() {
            if !t.done {
                let frames = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];
                let f = frames[(t.at.elapsed().as_millis() / 100) as usize % frames.len()];
                let k = t.sources.len();
                let what = if k == 0 {
                    "Thinking…".to_string()
                } else {
                    format!("Reading {k} note{}…", if k == 1 { "" } else { "s" })
                };
                out.push(vec![sp(format!("{f} {what}"), faint)]);
            }
        } else if !answer.trim().is_empty() {
            let fresh = t.cache.as_ref().filter(|c| c.0 == answer.len() && c.1 == w && c.2 == tg).map(|c| c.3.clone());
            let r = match fresh {
                Some(r) => r,
                None => {
                    let r = std::rc::Rc::new(crate::md::render(&crate::vault::Note::scratch(&answer), &app.vault, &app.t, w, &[]));
                    app.chat.turns[i].cache = Some((answer.len(), w, tg, r.clone()));
                    r
                }
            };
            let t = &app.chat.turns[i];
            let mut ls = r.lines.clone();
            if !t.done
                && let Some(last) = ls.last_mut()
            {
                last.push(sp("▍", acc));
            }
            out.extend(ls);
        }
        let t = &app.chat.turns[i];
        if let Some(e) = &t.err {
            if !answer.trim().is_empty() {
                out.push(vec![]);
            }
            para(&mut out, vec![sp(e.clone(), flame)]);
        }
        // the notes it was given
        if t.done && !t.sources.is_empty() {
            out.push(vec![]);
            let mut segs = vec![sp("Looked at  ", faint)];
            for (j, (k, name)) in crate::ask::names(&app.vault, &t.sources).into_iter().enumerate() {
                if j > 0 {
                    segs.push(sp(" · ", faint));
                }
                segs.push(sp(name, dim).on(Act::Open { key: k, heading: None }));
            }
            para(&mut out, segs);
        }
    }
    out
}

/// The built-in editor, in the note's place.
fn draw_editor(app: &mut App, p: &mut P, r: Rect) {
    use crate::editor::{Tint, ew, tints};
    let (ink, faint, dim, acc, line) = (app.t.c("ink"), app.t.c("faint"), app.t.c("dim"), app.t.c("accent"), app.t.c("line"));
    let (amber, flame, cyan, sea) = (app.t.c("amber"), app.t.c("flame"), app.t.c("cyan"), app.t.c("sea"));
    let sel_bg = app.t.c("raise");
    let name = app.vault.note(&app.editor.as_ref().unwrap().key).map(|n| n.name.clone()).unwrap_or_default();
    let ed = app.editor.as_mut().unwrap();
    let title = format!("Editing: {name}");
    let sub = if ed.modified { "● not saved" } else { "saved" };
    let old = p.bg;
    p.frame(r, acc, Some((&title, ink, true)), Some((sub, if ed.modified { amber } else { faint })));
    let inn = Rect::new(r.x + 2, r.y + 1, r.width.saturating_sub(4), r.height.saturating_sub(2));
    if inn.width < 6 || inn.height < 4 {
        return;
    }
    // where the cursor is, and the file's line endings
    let mut head = vec![sp(format!("line {}, col {}", ed.row + 1, ed.col + 1), faint)];
    if ed.crlf {
        head.push(sp("  ·  CRLF", faint));
    }
    if let Some(s) = ed.selected() {
        let n = s.chars().count();
        head.push(sp(format!("  ·  {n} selected"), faint));
    }
    p.line(inn.x, inn.y, &head, inn.right(), p.bg);
    let find_h = if ed.find.is_some() { 2 } else { 0 };
    let body = Rect::new(inn.x, inn.y + 2, inn.width, inn.height.saturating_sub(2 + find_h));
    let tw = body.width.saturating_sub(2) as usize;
    let h = body.height as usize;
    ed.width = tw;
    ed.height = h;
    let lay = ed.layout(tw);
    if ed.follow {
        ed.keep_visible(&lay, h);
        ed.follow = false;
    }
    ed.scroll = ed.scroll.min(lay.len().saturating_sub(1));
    let tint = tints(&ed.lines);
    let sel = ed.sel();
    let (cvr, cx) = ed.vis(&lay, ed.row, ed.col);
    for (i, vr) in lay.iter().enumerate().skip(ed.scroll).take(h) {
        let y = body.y + (i - ed.scroll) as u16;
        let l = &ed.lines[vr.line];
        let mut x = body.x;
        for (c, &ch) in l.iter().enumerate().take(vr.end).skip(vr.start) {
            let t = tint[vr.line].get(c).copied().unwrap_or(Tint::Plain);
            let fg = match t {
                Tint::Plain => ink,
                Tint::Heading => acc,
                Tint::Link => sea,
                Tint::Tag => cyan,
                Tint::Code => amber,
                Tint::Marker => faint,
                Tint::Front => dim,
            };
            let on = sel.is_some_and(|(a, b)| (vr.line, c) >= a && (vr.line, c) < b);
            let mut st = Style::default().fg(fg).bg(if on { sel_bg } else { p.bg });
            if t == Tint::Heading {
                st = st.add_modifier(Modifier::BOLD);
            }
            let s = if ch == '\t' { " ".repeat(ew(ch)) } else { ch.to_string() };
            x = p.put(x, y, &s, st, body.x + tw as u16 + 1);
        }
        // a selected line break shows as one cell
        if sel.is_some_and(|(a, b)| (vr.line, vr.end) >= a && (vr.line, vr.end) < b) && vr.end == l.len() {
            p.put(x, y, " ", Style::default().bg(sel_bg), body.x + tw as u16 + 1);
        }
        if i == cvr {
            let cx = body.x + cx as u16;
            let under = l.get(ed.col).filter(|_| ed.col < vr.end || vr.end == l.len()).copied();
            let under = match under {
                Some('\t') | None => " ".to_string(),
                Some(c) => c.to_string(),
            };
            p.put(cx, y, &under, Style::default().add_modifier(Modifier::REVERSED).fg(ink), body.x + tw as u16 + 1);
        }
    }
    if lay.len() > h {
        p.scrollbar(body.right(), body.y, body.height, lay.len(), ed.scroll, faint, line);
    }
    app.edit_area = Rect::new(body.x, body.y, tw as u16, body.height);
    // the find box
    if let Some(f) = ed.find.as_mut() {
        let fy = body.bottom() + 1;
        let soft = app.t.c("line-soft");
        p.put(inn.x, fy - 1, &"─".repeat(inn.width as usize), Style::default().fg(soft).bg(p.bg), inn.right());
        let x = p.put(inn.x, fy, "Find ", Style::default().fg(acc).bg(p.bg).add_modifier(Modifier::BOLD), inn.right());
        let tail = if ed.find_miss { "  no match" } else { "  enter next · shift+enter back · esc close" };
        let fw = inn.width.saturating_sub(5 + crate::util::cell_len(tail) as u16).max(8);
        input_box(p, f, x, fy, fw, "type to find", true, ink, faint, acc);
        p.put(x + fw, fy, tail, Style::default().fg(if ed.find_miss { flame } else { faint }).bg(p.bg), inn.right());
    }
    p.bg = old;
}

fn draw_menu(app: &mut App, p: &mut P) {
    let (w, h) = app.size;
    let (panel, line, faint, ink, sel, raise) =
        (app.t.c("panel"), app.t.c("line"), app.t.c("faint"), app.t.c("ink"), app.t.c("sel"), app.t.c("raise"));
    p.hit(Rect::new(0, 0, w, h), HitKind::Backdrop);
    let m = app.modals.last_mut().unwrap();
    let widths = m.opts.iter().flatten().map(|(_, s)| line_width(s) + 4);
    let mw = widths.chain([22, cell_len(&m.title) + 6]).max().unwrap_or(22).min(w as usize) as u16;
    let mh = (m.opts.len() as u16 + 2).min(h.saturating_sub(1));
    let mx = m.x.min(w as i32 - mw as i32 - 1).max(0) as u16;
    let my = if (m.y + mh as i32) < h as i32 { m.y } else { (m.y - mh as i32 + 1).max(0) }.max(0) as u16;
    let r = Rect::new(mx, my, mw.min(w.saturating_sub(mx)), mh.min(h.saturating_sub(my)));
    p.fill(r, panel);
    let old = p.bg;
    p.bg = panel;
    p.frame(r, faint, Some((&m.title, faint, false)), None);
    let inn = inner(r);
    let rows = inn.height as usize;
    if let Some(hv) = m.hi {
        if hv < m.top {
            m.top = hv;
        } else if hv >= m.top + rows {
            m.top = hv + 1 - rows;
        }
    }
    m.top = m.top.min(m.opts.len().saturating_sub(rows));
    p.hit(r, HitKind::Scroll(Scroll::Menu));
    for (oi, o) in m.opts.iter().enumerate().skip(m.top).take(rows) {
        let oy = inn.y + (oi - m.top) as u16;
        let rr = Rect::new(inn.x, oy, inn.width, 1);
        match o {
            None => {
                p.put(inn.x, oy, &"─".repeat(inn.width as usize), Style::default().fg(line).bg(panel), inn.right());
            }
            Some((k, segs)) => {
                let disabled = k == "-none";
                let obg = if m.hi == Some(oi) {
                    sel
                } else if p.hovering(rr) && !disabled {
                    raise
                } else {
                    panel
                };
                p.fill(rr, obg);
                let segs: Vec<Seg> = segs
                    .iter()
                    .map(|s| {
                        let mut s = s.clone();
                        if s.fg.is_none() {
                            s.fg = Some(ink);
                        }
                        s
                    })
                    .collect();
                p.line(inn.x + 1, oy, &segs, inn.right() - 1, obg);
                if !disabled {
                    p.hit(rr, HitKind::Act(Act::MenuPick(oi)));
                }
            }
        }
    }
    if m.opts.len() > rows {
        p.scrollbar(inn.right() - 1, inn.y, inn.height, m.opts.len(), m.top, faint, line);
    }
    p.bg = old;
}

fn draw_toasts(app: &mut App, p: &mut P) {
    let (w, h) = app.size;
    let tw = 60.min(w / 2).max(20).min(w);
    let mut bottom = h.saturating_sub(1);
    let (panel, acc, ink, flame) = (app.t.c("panel"), app.t.c("accent"), app.t.c("ink"), app.t.c("flame"));
    for t in app.toasts.iter().rev() {
        let mut segs = vec![];
        if !t.title.is_empty() {
            segs.push(sp(t.title.clone(), acc).b());
            segs.push(plain("\n"));
        }
        segs.push(sp(t.msg.clone(), ink));
        let lines = wrap(&segs, tw.saturating_sub(3) as usize);
        let th = lines.len() as u16 + 2;
        if bottom < th + 1 {
            break;
        }
        let y0 = bottom - th;
        let x0 = w.saturating_sub(tw + 1);
        let r = Rect::new(x0, y0, tw, th);
        p.fill(r, panel);
        let bar = if t.sev == Sev::Error { flame } else { acc };
        for yy in r.y..r.bottom() {
            p.put(r.x, yy, "▌", Style::default().fg(bar).bg(panel), r.x + 1);
        }
        for (i, l) in lines.iter().enumerate() {
            p.line(r.x + 2, r.y + 1 + i as u16, l, r.right() - 1, panel);
        }
        p.hit(r, HitKind::Act(Act::Toast(t.id)));
        bottom = y0.saturating_sub(1);
    }
}

impl App {
    /// The key hints along the bottom; the ones that do something click.
    pub fn keybar(&self) -> Line {
        let (ink, faint) = (self.t.c("dim"), self.t.c("faint"));
        let k = |key: &str, what: &str, a: Option<Act>| -> Vec<Seg> {
            let mut v = vec![sp(key, ink), sp(format!(" {what}"), faint)];
            if let Some(a) = a {
                v = v.into_iter().map(|s| s.on(a.clone())).collect();
            }
            v.push(plain("   "));
            v
        };
        let mut out = vec![];
        if let Some(ed) = &self.editor {
            let n = |key: &str, what: &str| vec![sp(key, ink), sp(format!(" {what}"), faint), plain("  ")];
            if ed.find.is_some() {
                out.extend(n("enter", "next"));
                out.extend(n("shift+enter", "back"));
                out.extend(n("esc", "close find"));
                out.extend(n("^S", "save"));
                return out;
            }
            out.extend(n("^S", "save"));
            out.extend(n("esc", "done"));
            out.extend(n("^F", "find"));
            out.extend(n("^Z", "undo"));
            out.extend(n("^Y", "redo"));
            out.extend(n("^C", "copy"));
            out.extend(n("^X", "cut"));
            out.extend(n("^V", "paste"));
            out.extend(n("^A", "all"));
            out.extend(n("shift+arrows", "select"));
            return out;
        }
        if self.asking {
            let n = |key: &str, what: &str| vec![sp(key, ink), sp(format!(" {what}"), faint), plain("  ")];
            if self.chat.busy() {
                out.extend(n("esc", "stop"));
            } else {
                out.extend(n("enter", "ask"));
                out.extend(n("esc", "close"));
            }
            out.extend(n("↑↓ pgup pgdn", "scroll"));
            out.extend(k("^O", "model", Some(Act::AskModel)));
            out.extend(k("^N", "new chat", Some(Act::AskNew)));
            return out;
        }
        if self.focus == Focus::Filter {
            out.extend(k("enter", "search every note", Some(Act::Search)));
            out.extend(k("↑↓", "move", None));
            out.extend(k("esc", "clear", None));
            return out;
        }
        if let Some(whole) = self.graph_active() {
            out.extend(k("↑↓←→", "dots", None));
            out.extend(k("enter", "open", None));
            if whole {
                out.extend(k("+ − wheel", "zoom", None));
                out.extend(k("drag", "pan", None));
            } else {
                out.extend(k("+ −", "depth", None));
            }
            out.extend(k("0", "reset", Some(Act::GraphReset)));
            out.extend(k("esc", "back", None));
            out.extend(k("q", "quit", Some(Act::Quit)));
            return out;
        }
        if self.narrow() && self.focus == Focus::None {
            match self.shown() {
                Stage::Rail => {
                    out.extend(k("↑↓", "pick", None));
                    out.extend(k("enter →", "its notes", None));
                    out.extend(k("←", "fold", None));
                    out.extend(k("/", "search", Some(Act::Filter)));
                    out.extend(k("q", "quit", Some(Act::Quit)));
                    return out;
                }
                Stage::List => {
                    out.extend(k("↑↓", "notes", None));
                    out.extend(k("enter →", "read", None));
                    out.extend(k("esc ←", "folders", Some(Act::StageBack)));
                    out.extend(k("/", "search", Some(Act::Filter)));
                    out.extend(k("?", "keys", Some(Act::Help)));
                    out.extend(k("q", "quit", Some(Act::Quit)));
                    return out;
                }
                Stage::Note => {
                    out.extend(k("↑↓", "scroll", None));
                    out.extend(k("tab", "links", None));
                    out.extend(k("esc ←", "notes", Some(Act::StageBack)));
                    out.extend(k("[ ]", "back", Some(Act::Back)));
                    out.extend(k("e", "edit", Some(Act::Edit)));
                    out.extend(k("E", "$EDITOR", Some(Act::EditOutside)));
                    out.extend(k("q", "quit", Some(Act::Quit)));
                    return out;
                }
            }
        }
        if self.focus == Focus::Rail {
            out.extend(k("↑↓", "sidebar", None));
            out.extend(k("← →", "fold / back to the notes", None));
            out.extend(k("esc", "back", None));
            return out;
        }
        out.extend(k("↑↓", "notes", None));
        out.extend(k("enter", "open", None));
        out.extend(k("/", "search", Some(Act::Filter)));
        out.extend(k("tab", "links", None));
        out.extend(k("[ ]", "back", Some(Act::Back)));
        if !self.terms.is_empty() {
            out.extend(k("n", "match", Some(Act::NextHit)));
        }
        out.extend(k("a", "ask", Some(Act::Ask)));
        out.extend(k("e", "edit", Some(Act::Edit)));
        out.extend(k("E", "$EDITOR", Some(Act::EditOutside)));
        out.extend(k("b", "sidebar", Some(Act::ToggleRail)));
        out.extend(k("?", "keys", Some(Act::Help)));
        out.extend(k("q", "quit", Some(Act::Quit)));
        out
    }
}

/// The screen as plain text, for tests and dumps.
pub fn text_of(buf: &Buffer) -> String {
    let a = buf.area();
    let mut out = String::new();
    for y in a.top()..a.bottom() {
        let mut line = String::new();
        for x in a.left()..a.right() {
            line.push_str(buf.cell((x, y)).map(|c| c.symbol()).unwrap_or(" "));
        }
        out.push_str(line.trim_end());
        out.push('\n');
    }
    out
}
