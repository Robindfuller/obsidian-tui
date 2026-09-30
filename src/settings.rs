//! The Settings panel: everything the ask pane can be told, each with a line
//! saying what it does. ↑↓ pick a setting, ←→ change it, enter edits text or
//! opens the list of models. Every change is saved as it's made.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};

use crate::app::App;
use crate::ask::Conf;
use crate::draw::{HitKind, P, input_box};
use crate::input::LineEdit;
use crate::rich::{Act, Line, sp, wrap};
use crate::util::fit;

pub struct Panel {
    pub row: usize,
    /// a text setting being typed into
    pub edit: Option<LineEdit>,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Row {
    Model,
    Host,
    Notes,
    Local,
    Claude,
    Ctx,
    Think,
    Turns,
    Extra,
    Reset,
}

pub const ROWS: [Row; 10] =
    [Row::Model, Row::Host, Row::Notes, Row::Local, Row::Claude, Row::Ctx, Row::Think, Row::Turns, Row::Extra, Row::Reset];

const LOCAL: &[usize] = &[4_000, 6_000, 8_000, 12_000, 16_000, 24_000, 32_000, 48_000, 64_000];
const CLAUDE: &[usize] = &[10_000, 20_000, 40_000, 60_000, 100_000, 150_000];
const CTX: &[usize] = &[2_048, 4_096, 8_192, 16_384, 32_768, 65_536, 131_072];

impl Row {
    pub fn label(self) -> &'static str {
        match self {
            Row::Model => "Model",
            Row::Host => "Ollama address",
            Row::Notes => "Notes per question",
            Row::Local => "Reading, own computer",
            Row::Claude => "Reading, Claude",
            Row::Ctx => "Ollama memory",
            Row::Think => "Think first",
            Row::Turns => "Follow-ups remember",
            Row::Extra => "Extra instructions",
            Row::Reset => "Reset to defaults",
        }
    }

    fn help(self) -> &'static str {
        match self {
            Row::Model => {
                "What answers your questions. A model on this computer (Ollama) keeps your notes private; Claude sends the question and the notes that match it to Claude, on your Claude plan. Enter shows the list."
            }
            Row::Host => {
                "Where Ollama is running. Leave it empty for this computer (or OLLAMA_HOST, if that's set). Enter to change it, for example http://192.168.1.20:11434."
            }
            Row::Notes => "How many notes go with each question, at most. More can find more, but there's more to read, so it's slower.",
            Row::Local => {
                "How much note text a model on this computer reads for each question, shared out between the notes. More gives it more to go on but takes longer. About four characters make a token."
            }
            Row::Claude => "How much note text Claude reads for each question. Claude reads quickly, so this can be much more.",
            Row::Ctx => {
                "How much an Ollama model can hold in mind at once, in tokens: the notes, the conversation and the answer all have to fit. Bigger uses more memory."
            }
            Row::Think => {
                "Let models that can reason (like qwen3.5) think before they answer. Better on hard questions, but much slower. Models that can't just answer."
            }
            Row::Turns => "How many earlier questions and answers go with a follow-up, so it knows what “it” and “that” mean.",
            Row::Extra => {
                "Anything to add to what the model is told, like “answer in bullet points” or “I'm a nurse, keep it clinical”. Enter to change it."
            }
            Row::Reset => "Put every setting here back as it was. The model stays.",
        }
    }

    /// Changed with ←→.
    fn steps(self) -> bool {
        matches!(self, Row::Notes | Row::Local | Row::Claude | Row::Ctx | Row::Think | Row::Turns)
    }
}

/// The next value along a list of choices, from wherever `cur` is.
fn step_in(list: &[usize], cur: usize, d: i64) -> usize {
    let i = list.iter().position(|&x| x >= cur).unwrap_or(list.len() - 1) as i64;
    // between two choices, the first step lands on the nearer one that way
    let i = if d < 0 && list[i as usize] > cur { i } else { i + d };
    list[i.clamp(0, list.len() as i64 - 1) as usize]
}

fn thousands(n: usize) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

impl App {
    pub fn open_settings(&mut self) {
        if self.editor.is_some() {
            return;
        }
        if self.ask_model.is_none() {
            self.find_model();
        }
        self.settings = Some(Panel { row: 0, edit: None });
        self.dirty = true;
    }

    pub fn close_settings(&mut self) {
        self.settings = None;
        self.dirty = true;
    }

    fn row(&self) -> Row {
        ROWS[self.settings.as_ref().map(|s| s.row).unwrap_or(0).min(ROWS.len() - 1)]
    }

    pub fn settings_key(&mut self, k: &KeyEvent) {
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        self.dirty = true;
        let Some(panel) = self.settings.as_mut() else { return };
        if let Some(ed) = panel.edit.as_mut() {
            match k.code {
                KeyCode::Enter => {
                    let v = ed.value().trim().to_string();
                    panel.edit = None;
                    match ROWS[panel.row] {
                        Row::Host => self.ask_conf.host = v,
                        Row::Extra => self.ask_conf.extra = v,
                        _ => {}
                    }
                    self.save_state();
                }
                KeyCode::Esc => panel.edit = None,
                _ => {
                    ed.key(k);
                }
            }
            return;
        }
        match k.code {
            KeyCode::Char('c') if ctrl => self.quit = true,
            KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char(',') => self.close_settings(),
            KeyCode::Char('o') if ctrl => self.close_settings(),
            KeyCode::Up | KeyCode::Char('k') | KeyCode::BackTab => panel.row = panel.row.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') | KeyCode::Tab => panel.row = (panel.row + 1).min(ROWS.len() - 1),
            KeyCode::Home => panel.row = 0,
            KeyCode::End => panel.row = ROWS.len() - 1,
            KeyCode::Left | KeyCode::Char('h') | KeyCode::Char('-') => {
                let r = self.row();
                self.step(r, -1);
            }
            KeyCode::Right | KeyCode::Char('l') | KeyCode::Char('+') | KeyCode::Char('=') => {
                let r = self.row();
                self.step(r, 1);
            }
            KeyCode::Enter | KeyCode::Char(' ') => {
                let r = self.row();
                self.activate(r);
            }
            _ => {}
        }
    }

    /// A click on a row: pick it, and do what enter does there.
    pub fn settings_row(&mut self, i: usize) {
        let Some(p) = self.settings.as_mut() else { return };
        p.edit = None;
        p.row = i.min(ROWS.len() - 1);
        let r = ROWS[p.row];
        if !r.steps() {
            self.activate(r);
        }
    }

    pub fn settings_step(&mut self, i: usize, d: i64) {
        let Some(p) = self.settings.as_mut() else { return };
        p.edit = None;
        p.row = i.min(ROWS.len() - 1);
        let r = ROWS[p.row];
        self.step(r, d);
    }

    fn activate(&mut self, r: Row) {
        match r {
            Row::Model => self.model_menu(),
            Row::Host | Row::Extra => {
                let v = if r == Row::Host { self.ask_conf.host.clone() } else { self.ask_conf.extra.clone() };
                if let Some(p) = self.settings.as_mut() {
                    p.edit = Some(LineEdit::new(&v));
                }
            }
            Row::Think => self.step(r, 1),
            Row::Reset => {
                self.ask_conf = Conf::default();
                self.save_state();
                self.toast("", "Settings are back as they were.", crate::app::Sev::Info);
            }
            _ => {}
        }
        self.dirty = true;
    }

    fn step(&mut self, r: Row, d: i64) {
        let c = &mut self.ask_conf;
        match r {
            Row::Notes => c.notes = (c.notes as i64 + d).clamp(1, 20) as usize,
            Row::Local => c.local_chars = step_in(LOCAL, c.local_chars, d),
            Row::Claude => c.claude_chars = step_in(CLAUDE, c.claude_chars, d),
            Row::Ctx => c.ctx = step_in(CTX, c.ctx, d),
            Row::Think => c.think = !c.think,
            Row::Turns => c.turns = (c.turns as i64 + d).clamp(0, 10) as usize,
            _ => return,
        }
        self.save_state();
        self.dirty = true;
    }

    /// A setting as shown: the value, and whether it's a quiet default.
    fn shown_value(&self, r: Row) -> (String, bool) {
        let c = &self.ask_conf;
        match r {
            Row::Model => match &self.ask_model {
                Some(m) => (format!("{} ▾", m.label), false),
                None => ("none yet ▾".into(), true),
            },
            Row::Host if c.host.is_empty() => match std::env::var("OLLAMA_HOST") {
                Ok(h) if !h.trim().is_empty() => (format!("{} (OLLAMA_HOST)", crate::ask::ollama_host("")), true),
                _ => ("this computer".into(), true),
            },
            Row::Host => (c.host.clone(), false),
            Row::Notes => (c.notes.to_string(), false),
            Row::Local => (format!("{} characters", thousands(c.local_chars)), false),
            Row::Claude => (format!("{} characters", thousands(c.claude_chars)), false),
            Row::Ctx => (format!("{} tokens", thousands(c.ctx)), false),
            Row::Think => ((if c.think { "on" } else { "off" }).into(), false),
            Row::Turns => (format!("{} {}", c.turns, if c.turns == 1 { "turn" } else { "turns" }), false),
            Row::Extra if c.extra.is_empty() => ("none".into(), true),
            Row::Extra => (c.extra.clone(), false),
            Row::Reset => (String::new(), false),
        }
    }

    /// Will the notes, the conversation and an answer fit Ollama's memory?
    fn fits(&self) -> bool {
        let c = &self.ask_conf;
        c.local_chars * 10 / 35 + 1_500 <= c.ctx
    }
}

/// The panel, over everything else.
pub fn draw_settings(app: &mut App, p: &mut P) {
    let Some(panel) = app.settings.as_ref() else { return };
    let row = panel.row;
    let editing = panel.edit.is_some();
    let (w, h) = app.size;
    let t = &app.t;
    let (panel_bg, ink, faint, dim, acc, raise, amber) =
        (t.c("panel"), t.c("ink"), t.c("faint"), t.c("dim"), t.c("accent"), t.c("raise"), t.c("amber"));
    let pw = 78.min(w.saturating_sub(2));
    let inner_w = pw.saturating_sub(4) as usize;
    let help = wrap(&[sp(ROWS[row].help(), dim)], inner_w);
    let warn: Vec<Line> = if app.fits() {
        vec![]
    } else {
        wrap(
            &[sp(
                format!(
                    "The notes may not fit in {} tokens: raise Ollama memory, or read less.",
                    thousands(app.ask_conf.ctx)
                ),
                amber,
            )],
            inner_w,
        )
    };
    let ph = (ROWS.len() as u16 + 6 + help.len() as u16 + warn.len() as u16).min(h.saturating_sub(1));
    let r = Rect::new(w.saturating_sub(pw) / 2, h.saturating_sub(ph + 1) / 2, pw, ph);
    // a click outside closes it; inside, only its own parts do anything
    p.hit(Rect::new(0, 0, w, h), HitKind::Act(Act::SetClose));
    p.fill(r, panel_bg);
    let old = p.bg;
    p.bg = panel_bg;
    let sub = if editing { "enter keep · esc undo" } else { "↑↓ pick · ←→ change · enter edit · esc close" };
    p.frame(r, faint, Some(("Settings", ink, true)), Some((sub, faint)));
    p.hit(r, HitKind::Blur);
    let x0 = r.x + 2;
    let right = r.right().saturating_sub(2);
    let mut y = r.y + 1;
    p.put(x0, y, "ASK YOUR NOTES", Style::default().fg(faint).bg(panel_bg).add_modifier(Modifier::BOLD), right);
    y += 1;
    let lw = 24usize;
    for (i, rw) in ROWS.iter().enumerate() {
        if y >= r.bottom().saturating_sub(1) {
            break;
        }
        let on = i == row;
        let rr = Rect::new(r.x + 1, y, r.width.saturating_sub(2), 1);
        let bg = if on { raise } else { panel_bg };
        p.fill(rr, bg);
        p.put(r.x + 1, y, if on { "▌" } else { " " }, Style::default().fg(acc).bg(bg), right);
        let mut st = Style::default().fg(if on { ink } else { dim }).bg(bg);
        if on {
            st = st.add_modifier(Modifier::BOLD);
        }
        p.hit(rr, HitKind::Act(Act::SetRow(i)));
        if *rw == Row::Reset {
            p.put(x0, y, rw.label(), Style::default().fg(if on { ink } else { dim }).bg(bg), right);
            y += 1;
            continue;
        }
        let vx = p.put(x0, y, &fit(rw.label(), lw), st, right);
        let vw = right.saturating_sub(vx) as usize;
        let editing_here = on && editing;
        if editing_here {
            let placeholder = if *rw == Row::Host { "empty: this computer" } else { "nothing extra" };
            if let Some(ed) = app.settings.as_mut().and_then(|s| s.edit.as_mut()) {
                input_box(p, ed, vx, y, vw as u16, placeholder, true, ink, faint, acc);
            }
            y += 1;
            continue;
        }
        let (v, quiet) = app.shown_value(*rw);
        let vcol = if quiet { faint } else { ink };
        if rw.steps() {
            let segs = vec![
                sp("‹ ", faint).bg(bg).on(Act::SetStep(i, -1)),
                sp(v, vcol).bg(bg),
                sp(" ›", faint).bg(bg).on(Act::SetStep(i, 1)),
            ];
            p.line(vx, y, &segs, right, bg);
        } else {
            let v = crate::util::crop(&v, vw);
            p.put(vx, y, &v, Style::default().fg(vcol).bg(bg), right);
        }
        y += 1;
    }
    // what the picked setting does
    y += 1;
    if y < r.bottom() {
        p.put(x0, y, &"─".repeat(inner_w), Style::default().fg(t.c("line-soft")).bg(panel_bg), right);
    }
    y += 1;
    for l in help.iter().chain(warn.iter()) {
        if y >= r.bottom().saturating_sub(1) {
            break;
        }
        p.line(x0, y, l, right, panel_bg);
        y += 1;
    }
    p.bg = old;
}
