//! A note's Markdown, drawn as terminal lines: headings, emphasis, lists and
//! tasks, quotes and Obsidian callouts, code, tables, rules, frontmatter and
//! #tags, with every link clickable. Rendered for one width at a time, since
//! quote bars and list indents have to repeat on each wrapped line.

use std::sync::LazyLock;

use pulldown_cmark::{Alignment, CodeBlockKind, Event, HeadingLevel, LinkType, Options, Parser, Tag, TagEnd};
use ratatui::style::Color;
use regex::Regex;

use crate::rich::{Act, Line, Seg, line_width, plain, sp, wrap};
use crate::theme::{Theme, color, tmix};
use crate::util::{cell_len, char_w};
use crate::vault::{Note, TAG, Vault, find_ci, is_url};

/// Private-use markers a callout's first line is rewritten to before parsing.
const CALLOUT: char = '\u{E000}';
const CALLOUT_SEP: char = '\u{E001}';

static CALLOUT_LINE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?m)^([ \t]*(?:>[ \t]?)+)\[!([A-Za-z][\w-]*)\][+-]?[ \t]*(.*)$").unwrap());
static COMMENT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?s)%%.*?%%").unwrap());
static HIGHLIGHT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"==([^=\n]+)==").unwrap());

pub struct Rendered {
    pub lines: Vec<Line>,
    /// (level, text, line index)
    pub headings: Vec<(u8, String, usize)>,
    /// every link in reading order, for tab to step through: (line, action)
    pub links: Vec<(usize, Act)>,
    /// lines with a search match
    pub hits: Vec<usize>,
}

enum Cont {
    Quote { col: Color },
    List { next: Option<u64> },
    Item { bullet: Line, width: usize, used: bool, done: bool },
}

struct Table {
    aligns: Vec<Alignment>,
    rows: Vec<Vec<Line>>,
    row: Vec<Line>,
    head_rows: usize,
}

struct R<'a> {
    t: &'a Theme,
    vault: &'a Vault,
    from: &'a str,
    width: usize,
    out: Vec<Line>,
    blank: Vec<bool>,
    cur: Vec<Seg>,
    conts: Vec<Cont>,
    bold: usize,
    italic: usize,
    strike: usize,
    heading: Option<u8>,
    link: Option<(Act, Color)>,
    image: Option<(String, LinkType)>,
    code: Option<(String, String)>,
    table: Option<Table>,
    pending_gap: bool,
    fresh_item: bool,
    marker_para: bool,
    headings: Vec<(u8, String, usize)>,
}

/// Callout kinds to a theme colour and a label.
fn callout_style(t: &Theme, kind: &str) -> (Color, &'static str) {
    match kind.to_lowercase().as_str() {
        "abstract" | "summary" | "tldr" => (t.c("cyan"), "Summary"),
        "info" => (t.c("s2"), "Info"),
        "todo" => (t.c("s2"), "Todo"),
        "tip" | "hint" | "important" => (t.c("cyan"), "Tip"),
        "success" | "check" | "done" => (t.c("teal"), "Done"),
        "question" | "help" | "faq" => (t.c("amber"), "Question"),
        "warning" | "caution" | "attention" => (t.c("amber"), "Warning"),
        "failure" | "fail" | "missing" => (t.c("flame"), "Failed"),
        "danger" | "error" => (t.c("flame"), "Danger"),
        "bug" => (t.c("flame"), "Bug"),
        "example" => (t.c("s5"), "Example"),
        "quote" | "cite" => (t.c("dim"), "Quote"),
        _ => (t.c("accent"), "Note"),
    }
}

fn title_case(s: &str) -> String {
    let mut c = s.chars();
    c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
}

/// Cut a line of segments to `w` cells, with an ellipsis if it didn't fit.
pub fn crop_line(l: &[Seg], w: usize) -> Line {
    if line_width(l) <= w {
        return l.to_vec();
    }
    let mut out: Line = vec![];
    let mut used = 0;
    'outer: for s in l {
        let mut piece = s.clone();
        piece.text.clear();
        for c in s.text.chars() {
            if used + char_w(c) > w.saturating_sub(1) {
                if !piece.text.is_empty() {
                    out.push(piece);
                }
                break 'outer;
            }
            used += char_w(c);
            piece.text.push(c);
        }
        out.push(piece);
    }
    let fg = l.last().and_then(|s| s.fg);
    out.push(Seg { text: "…".into(), fg, ..Seg::default() });
    out
}

/// Mark the given char ranges of a line (search hits) with `bg`/`fg`.
pub fn mark(l: &[Seg], spans: &[(usize, usize)], bg: Color, fg: Color) -> Line {
    let mut out: Line = vec![];
    let mut i = 0;
    for s in l {
        let mut run = Seg { text: String::new(), ..s.clone() };
        let mut run_on = None;
        for c in s.text.chars() {
            let on = spans.iter().any(|&(a, b)| i >= a && i < b);
            if run_on.is_some_and(|r| r != on) && !run.text.is_empty() {
                out.push(std::mem::replace(&mut run, Seg { text: String::new(), ..s.clone() }));
            }
            if on {
                run.bg = Some(bg);
                run.fg = Some(fg);
            } else {
                run.bg = s.bg;
                run.fg = s.fg;
            }
            run_on = Some(on);
            run.text.push(c);
            i += 1;
        }
        if !run.text.is_empty() {
            out.push(run);
        }
    }
    out
}

pub fn line_text(l: &[Seg]) -> String {
    l.iter().map(|s| s.text.as_str()).collect()
}

impl<'a> R<'a> {
    fn ink(&self) -> Color {
        let done = self.conts.iter().any(|c| matches!(c, Cont::Item { done: true, .. }));
        match self.heading {
            _ if done => self.t.c("faint"),
            Some(1) => self.t.c("accent"),
            Some(2) => self.t.c("s2"),
            Some(3) => self.t.c("s3"),
            Some(_) => self.t.c("dim"),
            None => self.t.c("ink"),
        }
    }

    fn seg(&self, text: &str) -> Seg {
        let done = self.conts.iter().any(|c| matches!(c, Cont::Item { done: true, .. }));
        let mut s = sp(text, self.ink());
        s.bold = self.bold > 0 || self.heading.is_some();
        s.italic = self.italic > 0;
        s.strike = self.strike > 0 || done;
        if self.strike > 0 {
            s.fg = Some(self.t.c("dim"));
        }
        if let Some((a, c)) = &self.link {
            s.fg = Some(*c);
            s.act = Some(a.clone());
        }
        s
    }

    /// Text, with #tags and ==highlights== picked out when not inside a link.
    fn text(&mut self, text: &str) {
        if self.link.is_some() {
            let s = self.seg(text);
            self.cur.push(s);
            return;
        }
        // ==highlight== first, then tags inside each piece
        let hl_bg = color(&tmix(self.t.h("panel"), self.t.h("amber"), 0.35));
        let mut at = 0;
        for m in HIGHLIGHT.captures_iter(text) {
            let whole = m.get(0).unwrap();
            self.tags(&text[at..whole.start()], None);
            self.tags(&m[1], Some(hl_bg));
            at = whole.end();
        }
        self.tags(&text[at..], None);
    }

    fn tags(&mut self, text: &str, bg: Option<Color>) {
        let mut at = 0;
        for c in TAG.captures_iter(text) {
            let g = c.get(1).unwrap();
            let hash = g.start() - 1;
            if hash > at {
                let mut s = self.seg(&text[at..hash]);
                s.bg = bg;
                self.cur.push(s);
            }
            let tag = g.as_str().to_string();
            let mut s = sp(format!("#{tag}"), self.t.c("cyan")).on(Act::Tag(tag));
            s.bg = bg;
            self.cur.push(s);
            at = g.end();
        }
        if at < text.len() {
            let mut s = self.seg(&text[at..]);
            s.bg = bg;
            self.cur.push(s);
        }
    }

    fn prefix(&self, first: bool) -> Line {
        let mut out = vec![];
        for c in &self.conts {
            match c {
                Cont::Quote { col } => out.push(sp("│ ", *col)),
                Cont::List { .. } => {}
                Cont::Item { bullet, width, used, .. } => {
                    if first && !used {
                        out.extend(bullet.iter().cloned());
                    } else {
                        out.push(plain(" ".repeat(*width)));
                    }
                }
            }
        }
        out
    }

    fn push_line(&mut self, l: Line, blank: bool) {
        self.out.push(l);
        self.blank.push(blank);
        if !blank {
            self.fresh_item = false;
            for c in self.conts.iter_mut() {
                if let Cont::Item { used, .. } = c {
                    *used = true;
                }
            }
        }
    }

    fn avail(&self) -> usize {
        self.width.saturating_sub(line_width(&self.prefix(true))).max(4)
    }

    fn flush(&mut self) {
        if self.cur.iter().all(|s| s.text.trim().is_empty()) {
            self.cur.clear();
            return;
        }
        let segs = std::mem::take(&mut self.cur);
        let w = self.avail();
        let start = self.out.len();
        for (i, l) in wrap(&segs, w).into_iter().enumerate() {
            let mut line = self.prefix(i == 0);
            line.extend(l);
            self.push_line(line, false);
        }
        if let Some(level) = self.heading {
            let text: String = line_text(&segs).trim().to_string();
            self.headings.push((level, text, start));
        }
    }

    fn gap(&mut self) {
        if self.out.is_empty() || self.blank.last() == Some(&true) {
            return;
        }
        // blank lines inside a quote keep the bar
        let bars: Line = self
            .conts
            .iter()
            .filter_map(|c| match c {
                Cont::Quote { col } => Some(sp("│", *col)),
                _ => None,
            })
            .collect();
        let mut l = self.prefix(false);
        if !bars.is_empty() {
            l = l.into_iter().map(|mut s| {
                s.text = s.text.trim_end().to_string();
                s
            }).collect();
        }
        self.push_line(l, true);
    }

    fn block_start(&mut self) {
        self.flush();
        if self.pending_gap && !self.fresh_item {
            self.gap();
        }
        self.pending_gap = false;
    }

    fn link_act(&self, dest: &str, lt: LinkType) -> (Act, Color) {
        let (accent, faint, sea) = (self.t.c("accent"), self.t.c("faint"), self.t.c("sea"));
        if matches!(lt, LinkType::Email) {
            return (Act::Url(format!("mailto:{dest}")), sea);
        }
        if !matches!(lt, LinkType::WikiLink { .. }) && is_url(dest) {
            return (Act::Url(dest.to_string()), sea);
        }
        let dest = if matches!(lt, LinkType::WikiLink { .. }) { dest.to_string() } else { crate::util::url_dec(dest) };
        let (target, heading) = match dest.split_once('#') {
            Some((t, h)) => (t.to_string(), Some(h.to_string()).filter(|h| !h.starts_with('^') && !h.is_empty())),
            None => (dest.clone(), None),
        };
        if let Some(i) = self.vault.resolve(&target, Some(self.from)) {
            return (Act::Open { key: self.vault.notes[i].key.clone(), heading }, accent);
        }
        if let Some(f) = self.vault.resolve_file(&target, Some(self.from)) {
            return (Act::File(f), sea);
        }
        (Act::Missing(target), faint)
    }

    fn emit_code(&mut self, lang: &str, code: &str) {
        let panel = self.t.c("panel");
        let (dim, faint) = (self.t.c("dim"), self.t.c("faint"));
        let w = self.avail();
        let inner = w.saturating_sub(2).max(1);
        if !lang.is_empty() {
            let mut l = self.prefix(true);
            l.push(sp(format!(" {}", crate::util::fit(lang, w.saturating_sub(1))), faint).bg(panel));
            self.push_line(l, false);
        }
        let code = code.strip_suffix('\n').unwrap_or(code).replace('\t', "    ");
        for src in code.split('\n') {
            // fold long lines rather than lose the end of them
            let mut pieces: Vec<String> = vec![];
            let mut cur = String::new();
            let mut used = 0;
            for c in src.chars() {
                if used + char_w(c) > inner {
                    pieces.push(std::mem::take(&mut cur));
                    used = 0;
                }
                cur.push(c);
                used += char_w(c);
            }
            pieces.push(cur);
            for p in pieces {
                let mut l = self.prefix(true);
                let pad = inner.saturating_sub(cell_len(&p));
                l.push(sp(format!(" {p}{} ", " ".repeat(pad)), dim).bg(panel));
                self.push_line(l, false);
            }
        }
    }

    fn emit_table(&mut self, tb: Table) {
        let line_c = self.t.c("line");
        let n = tb.rows.iter().map(|r| r.len()).max().unwrap_or(0);
        if n == 0 {
            return;
        }
        let mut widths: Vec<usize> =
            (0..n).map(|i| tb.rows.iter().filter_map(|r| r.get(i)).map(|c| line_width(c)).max().unwrap_or(0).max(1)).collect();
        let avail = self.avail().saturating_sub(3 * n + 1);
        while widths.iter().sum::<usize>() > avail {
            let (i, &m) = widths.iter().enumerate().max_by_key(|(_, w)| **w).unwrap();
            if m <= 3 {
                break;
            }
            widths[i] -= 1;
        }
        let rule = |l: &str, m: &str, r: &str| -> String {
            let mut s = l.to_string();
            for (i, w) in widths.iter().enumerate() {
                s.push_str(&"─".repeat(w + 2));
                s.push_str(if i + 1 == n { r } else { m });
            }
            s
        };
        let top = rule("╭", "┬", "╮");
        let mid = rule("├", "┼", "┤");
        let bot = rule("╰", "┴", "╯");
        let mut l = self.prefix(true);
        l.push(sp(top, line_c));
        self.push_line(l, false);
        for (ri, row) in tb.rows.iter().enumerate() {
            let mut l = self.prefix(true);
            l.push(sp("│", line_c));
            for (i, w) in widths.iter().enumerate() {
                let cell = row.get(i).cloned().unwrap_or_default();
                let mut cell = crop_line(&cell, *w);
                if ri < tb.head_rows {
                    for s in cell.iter_mut() {
                        s.bold = true;
                    }
                }
                let pad = w.saturating_sub(line_width(&cell));
                let (lp, rp) = match tb.aligns.get(i) {
                    Some(Alignment::Right) => (pad, 0),
                    Some(Alignment::Center) => (pad / 2, pad - pad / 2),
                    _ => (0, pad),
                };
                l.push(plain(format!(" {}", " ".repeat(lp))));
                l.extend(cell);
                l.push(plain(format!("{} ", " ".repeat(rp))));
                l.push(sp("│", line_c));
            }
            self.push_line(l, false);
            if ri + 1 == tb.head_rows && tb.rows.len() > tb.head_rows {
                let mut l = self.prefix(true);
                l.push(sp(mid.clone(), line_c));
                self.push_line(l, false);
            }
        }
        let mut l = self.prefix(true);
        l.push(sp(bot, line_c));
        self.push_line(l, false);
    }

    fn frontmatter(&mut self, note: &Note) {
        if note.front.is_empty() {
            return;
        }
        let (faint, dim) = (self.t.c("faint"), self.t.c("dim"));
        let kw = note.front.iter().map(|(k, _)| cell_len(k)).max().unwrap_or(0).min(14) + 2;
        for (k, vals) in &note.front {
            let mut segs = vec![sp(crate::util::fit(k, kw), faint)];
            let lk = k.to_lowercase();
            if lk == "tags" || lk == "tag" {
                let tags: Vec<String> =
                    vals.iter().flat_map(|v| v.split([',', ' ']).map(|t| t.trim().trim_start_matches('#').to_string())).filter(|t| !t.is_empty()).collect();
                for (i, t) in tags.into_iter().enumerate() {
                    if i > 0 {
                        segs.push(plain(" "));
                    }
                    segs.push(sp(format!("#{t}"), self.t.c("cyan")).on(Act::Tag(t)));
                }
            } else {
                segs.push(sp(vals.join(", "), dim));
            }
            for (i, l) in wrap(&segs, self.width.max(8)).into_iter().enumerate() {
                let mut line = if i == 0 { vec![] } else { vec![plain(" ".repeat(kw))] };
                line.extend(l);
                self.push_line(line, false);
            }
        }
        let rule = sp("─".repeat(self.width.min(40)), self.t.c("line-soft"));
        self.push_line(vec![rule], false);
        self.pending_gap = true;
    }

    fn event(&mut self, e: Event) {
        if let Some((_, code)) = &mut self.code {
            match e {
                Event::Text(t) => {
                    code.push_str(&t);
                    return;
                }
                Event::End(TagEnd::CodeBlock) => {
                    let (lang, code) = self.code.take().unwrap();
                    self.emit_code(&lang, &code);
                    self.pending_gap = true;
                    return;
                }
                _ => return,
            }
        }
        if let Some((_, _)) = &self.image {
            match e {
                Event::End(TagEnd::Image) => {
                    let (dest, lt) = self.image.take().unwrap();
                    let (act, col) = self.link_act(&dest, lt);
                    let label = dest.rsplit('/').next().unwrap_or(&dest).to_string();
                    let label = crate::util::url_dec(&label);
                    self.cur.push(sp(format!("↳ {label}"), col).on(act));
                }
                _ => return,
            }
            return;
        }
        match e {
            Event::Start(tag) => self.start(tag),
            Event::End(tag) => self.end(tag),
            Event::Text(t) => {
                if let Some(rest) = t.strip_prefix(CALLOUT) {
                    let (kind, title) = rest.split_once(CALLOUT_SEP).unwrap_or((rest, ""));
                    let (col, label) = callout_style(self.t, kind);
                    if let Some(Cont::Quote { col: c }) = self.conts.iter_mut().rev().find(|c| matches!(c, Cont::Quote { .. })) {
                        *c = col;
                    }
                    let title = if title.trim().is_empty() {
                        if label == "Note" { title_case(kind) } else { label.to_string() }
                    } else {
                        title.trim().to_string()
                    };
                    let mut l = self.prefix(true);
                    l.push(sp(title, col).b());
                    self.push_line(l, false);
                    self.marker_para = true;
                    return;
                }
                if self.table.is_none() && self.cur.is_empty() && self.marker_para {
                    return;
                }
                self.text(&t);
            }
            Event::Code(c) => {
                let mut s = sp(c.to_string(), self.t.soft("amber", 0.75)).bg(self.t.c("panel"));
                if let Some((a, _)) = &self.link {
                    s.act = Some(a.clone());
                }
                self.cur.push(s);
            }
            Event::SoftBreak => self.cur.push(plain(" ")),
            Event::HardBreak => self.cur.push(plain("\n")),
            Event::Rule => {
                self.block_start();
                let w = self.avail();
                let mut l = self.prefix(true);
                l.push(sp("─".repeat(w), self.t.c("line")));
                self.push_line(l, false);
                self.pending_gap = true;
            }
            Event::Html(h) | Event::InlineHtml(h) => {
                let h = h.trim_end_matches('\n');
                if h.eq_ignore_ascii_case("<br>") || h.eq_ignore_ascii_case("<br/>") || h.eq_ignore_ascii_case("<br />") {
                    self.cur.push(plain("\n"));
                } else if !h.trim().is_empty() {
                    self.cur.push(sp(h.to_string(), self.t.c("faint")));
                }
            }
            Event::TaskListMarker(done) => {
                let (faint, teal) = (self.t.c("faint"), self.t.c("teal"));
                if let Some(Cont::Item { bullet, width, done: d, .. }) =
                    self.conts.iter_mut().rev().find(|c| matches!(c, Cont::Item { .. }))
                {
                    *bullet = vec![if done { sp("[x] ", teal) } else { sp("[ ] ", faint) }];
                    *width = 4;
                    *d = done;
                }
            }
            _ => {}
        }
    }

    fn start(&mut self, tag: Tag) {
        match tag {
            Tag::Paragraph => {
                if self.table.is_none() {
                    self.block_start();
                }
            }
            Tag::Heading { level, .. } => {
                self.flush();
                self.gap();
                self.pending_gap = false;
                self.heading = Some(match level {
                    HeadingLevel::H1 => 1,
                    HeadingLevel::H2 => 2,
                    HeadingLevel::H3 => 3,
                    HeadingLevel::H4 => 4,
                    HeadingLevel::H5 => 5,
                    HeadingLevel::H6 => 6,
                });
            }
            Tag::BlockQuote(_) => {
                self.block_start();
                self.conts.push(Cont::Quote { col: self.t.c("faint") });
            }
            Tag::CodeBlock(kind) => {
                self.block_start();
                let lang = match kind {
                    CodeBlockKind::Fenced(l) => l.split_whitespace().next().unwrap_or("").to_string(),
                    CodeBlockKind::Indented => String::new(),
                };
                self.code = Some((lang, String::new()));
            }
            Tag::List(start) => {
                let nested = self.conts.iter().any(|c| matches!(c, Cont::Item { .. }));
                if nested {
                    self.flush();
                } else {
                    self.block_start();
                }
                self.conts.push(Cont::List { next: start });
            }
            Tag::Item => {
                self.flush();
                let depth = self.conts.iter().filter(|c| matches!(c, Cont::List { .. })).count();
                let (faint, acc) = (self.t.c("faint"), self.t.c("accent"));
                let bullet = match self.conts.iter_mut().rev().find(|c| matches!(c, Cont::List { .. })) {
                    Some(Cont::List { next: Some(n) }) => {
                        let b = format!("{n}. ");
                        *n += 1;
                        vec![sp(b, faint)]
                    }
                    _ => vec![sp(
                        match depth {
                            0 | 1 => "• ",
                            2 => "◦ ",
                            _ => "▪ ",
                        },
                        acc,
                    )],
                };
                let width = line_width(&bullet);
                self.conts.push(Cont::Item { bullet, width, used: false, done: false });
                self.fresh_item = true;
            }
            Tag::Table(aligns) => {
                self.block_start();
                self.table = Some(Table { aligns, rows: vec![], row: vec![], head_rows: 0 });
            }
            Tag::TableHead | Tag::TableRow => {
                if let Some(t) = &mut self.table {
                    t.row.clear();
                }
            }
            Tag::TableCell => self.cur.clear(),
            Tag::Emphasis => self.italic += 1,
            Tag::Strong => self.bold += 1,
            Tag::Strikethrough => self.strike += 1,
            Tag::Link { link_type, dest_url, .. } => {
                self.link = Some(self.link_act(&dest_url, link_type));
            }
            Tag::Image { link_type, dest_url, .. } => {
                self.image = Some((dest_url.to_string(), link_type));
            }
            _ => {}
        }
    }

    fn end(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Paragraph => {
                if self.marker_para {
                    self.marker_para = false;
                    self.cur.clear();
                    self.pending_gap = false;
                    return;
                }
                if self.table.is_none() {
                    self.flush();
                    self.pending_gap = true;
                }
            }
            TagEnd::Heading(level) => {
                self.flush();
                if level == HeadingLevel::H1 {
                    let w = self.avail();
                    let mut l = self.prefix(true);
                    l.push(sp("─".repeat(w), self.t.c("line")));
                    self.push_line(l, false);
                }
                self.heading = None;
                self.pending_gap = true;
            }
            TagEnd::BlockQuote(_) => {
                self.flush();
                self.conts.pop();
                self.pending_gap = true;
            }
            TagEnd::List(_) => {
                self.flush();
                self.conts.pop();
                self.pending_gap = true;
            }
            TagEnd::Item => {
                self.flush();
                self.conts.pop();
                self.pending_gap = false;
            }
            TagEnd::TableCell => {
                let cell = std::mem::take(&mut self.cur);
                if let Some(t) = &mut self.table {
                    t.row.push(cell);
                }
            }
            TagEnd::TableHead => {
                if let Some(t) = &mut self.table {
                    let row = std::mem::take(&mut t.row);
                    t.rows.push(row);
                    t.head_rows = t.rows.len();
                }
            }
            TagEnd::TableRow => {
                if let Some(t) = &mut self.table {
                    let row = std::mem::take(&mut t.row);
                    t.rows.push(row);
                }
            }
            TagEnd::Table => {
                if let Some(t) = self.table.take() {
                    self.emit_table(t);
                }
                self.pending_gap = true;
            }
            TagEnd::Emphasis => self.italic = self.italic.saturating_sub(1),
            TagEnd::Strong => self.bold = self.bold.saturating_sub(1),
            TagEnd::Strikethrough => self.strike = self.strike.saturating_sub(1),
            TagEnd::Link => self.link = None,
            _ => {}
        }
    }
}

/// Obsidian-only syntax, turned into something the parser keeps: callout first
/// lines become a marker paragraph, %%comments%% go.
fn prepare(body: &str) -> String {
    let body = COMMENT.replace_all(body, "");
    CALLOUT_LINE
        .replace_all(&body, |c: &regex::Captures| {
            let bars = &c[1];
            format!("{bars}{CALLOUT}{}{CALLOUT_SEP}{}\n{}", &c[2], &c[3], bars.trim_end())
        })
        .into_owned()
}

pub fn render(note: &Note, vault: &Vault, t: &Theme, width: usize, terms: &[String]) -> Rendered {
    let mut r = R {
        t,
        vault,
        from: &note.key,
        width: width.max(8),
        out: vec![],
        blank: vec![],
        cur: vec![],
        conts: vec![],
        bold: 0,
        italic: 0,
        strike: 0,
        heading: None,
        link: None,
        image: None,
        code: None,
        table: None,
        pending_gap: false,
        fresh_item: false,
        marker_para: false,
        headings: vec![],
    };
    r.frontmatter(note);
    let src = prepare(note.body());
    let mut o = Options::empty();
    o.insert(Options::ENABLE_TABLES);
    o.insert(Options::ENABLE_STRIKETHROUGH);
    o.insert(Options::ENABLE_TASKLISTS);
    o.insert(Options::ENABLE_WIKILINKS);
    for e in Parser::new_ext(&src, o) {
        r.event(e);
    }
    r.flush();
    while r.blank.last() == Some(&true) {
        r.out.pop();
        r.blank.pop();
    }
    let mut lines = r.out;
    let mut hits = vec![];
    if !terms.is_empty() {
        let bg = t.c("amber");
        let fg = color(t.h("bg"));
        for (i, l) in lines.iter_mut().enumerate() {
            let spans = find_ci(&line_text(l), terms);
            if !spans.is_empty() {
                *l = mark(l, &spans, bg, fg);
                hits.push(i);
            }
        }
    }
    let mut links = vec![];
    for (i, l) in lines.iter().enumerate() {
        let mut last: Option<&Act> = None;
        for s in l {
            let is_link = matches!(s.act, Some(Act::Open { .. } | Act::Missing(_) | Act::Url(_) | Act::File(_) | Act::Tag(_)));
            if is_link && s.act.as_ref() != last {
                links.push((i, s.act.clone().unwrap()));
            }
            last = if is_link { s.act.as_ref() } else { None };
        }
    }
    Rendered { lines, headings: r.headings, links, hits }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn callouts_become_markers() {
        let p = prepare("> [!warning]- Careful\n> body");
        assert!(p.contains(CALLOUT));
        assert!(p.contains("Careful\n>\n> body"));
    }

    #[test]
    fn comments_go() {
        assert_eq!(prepare("a %%hidden%% b"), "a  b");
    }

    #[test]
    fn crop_adds_an_ellipsis() {
        let l = vec![plain("hello world")];
        assert_eq!(line_text(&crop_line(&l, 6)), "hello…");
    }

    #[test]
    fn marks_split_segments() {
        let l = vec![plain("find the plan")];
        let m = mark(&l, &[(9, 13)], Color::Red, Color::Black);
        assert_eq!(m.len(), 2);
        assert_eq!(m[1].text, "plan");
        assert_eq!(m[1].bg, Some(Color::Red));
    }
}
