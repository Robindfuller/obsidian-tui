//! Driving the app without a terminal: render to a buffer, press keys, click
//! on text. The tests use it, and `obsidian-tui --dump` prints what it sees.

use std::path::Path;

use ratatui::buffer::Buffer;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::Rect;

use crate::app::App;
use crate::draw::{draw, text_of};

pub struct H {
    pub app: App,
    pub buf: Buffer,
}

impl H {
    /// With its own state file, so tests don't share (or touch) the real one.
    pub fn new(vault: &Path, state: &Path, w: u16, h: u16) -> H {
        let mut app = App::with_state(vault, state.to_path_buf());
        app.headless = true;
        app.size = (w, h);
        let mut hh = H { app, buf: Buffer::empty(Rect::new(0, 0, w, h)) };
        hh.render();
        hh
    }

    pub fn render(&mut self) -> String {
        self.buf.reset();
        draw(&mut self.app, &mut self.buf);
        text_of(&self.buf)
    }

    pub fn press(&mut self, code: KeyCode, mods: KeyModifiers) -> String {
        self.app.on_key(KeyEvent::new(code, mods));
        self.render()
    }

    pub fn key(&mut self, code: KeyCode) -> String {
        self.press(code, KeyModifiers::NONE)
    }

    pub fn typ(&mut self, s: &str) -> String {
        for c in s.chars() {
            self.app.on_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
        }
        self.render()
    }

    pub fn click(&mut self, x: u16, y: u16) -> String {
        self.render();
        self.app.click(x, y, MouseButton::Left);
        self.render()
    }

    pub fn rclick(&mut self, x: u16, y: u16) -> String {
        self.render();
        self.app.click(x, y, MouseButton::Right);
        self.render()
    }

    pub fn wheel(&mut self, x: u16, y: u16, down: bool) -> String {
        self.render();
        let kind = if down { MouseEventKind::ScrollDown } else { MouseEventKind::ScrollUp };
        self.app.on_mouse(MouseEvent { kind, column: x, row: y, modifiers: KeyModifiers::NONE });
        self.render()
    }

    /// Where `needle` first shows on screen, searching from the top, or only
    /// right of column `from_x`.
    pub fn find_from(&mut self, needle: &str, from_x: u16) -> Option<(u16, u16)> {
        let text = self.render();
        for (y, line) in text.lines().enumerate() {
            let mut start = 0;
            while let Some(i) = line[start..].find(needle) {
                let x = crate::util::cell_len(&line[..start + i]) as u16;
                if x >= from_x {
                    return Some((x, y as u16));
                }
                start += i + needle.len();
            }
        }
        None
    }

    pub fn find(&mut self, needle: &str) -> Option<(u16, u16)> {
        self.find_from(needle, 0)
    }

    pub fn click_text(&mut self, needle: &str) -> String {
        let (x, y) = self.find(needle).unwrap_or_else(|| panic!("{needle:?} is not on screen:\n{}", self.render()));
        self.click(x, y)
    }

    /// Click text in the note pane (right of the list).
    pub fn click_in_note(&mut self, needle: &str) -> String {
        let from = self.app.geo().note.map(|r| r.x).unwrap_or(0);
        let (x, y) = self
            .find_from(needle, from)
            .unwrap_or_else(|| panic!("{needle:?} is not in the note:\n{}", self.render()));
        self.click(x, y)
    }

    /// Run steps like "key:down;type:plan;click:60,4;text:Garden" (for --dump).
    pub fn steps(&mut self, steps: &str) {
        for s in steps.split(';').filter(|s| !s.is_empty()) {
            if let Some(k) = s.strip_prefix("key:") {
                for k in k.split(',') {
                    let code = match k {
                        "down" => KeyCode::Down,
                        "up" => KeyCode::Up,
                        "left" => KeyCode::Left,
                        "right" => KeyCode::Right,
                        "enter" => KeyCode::Enter,
                        "escape" | "esc" => KeyCode::Esc,
                        "tab" => KeyCode::Tab,
                        "space" => KeyCode::Char(' '),
                        _ if k.chars().count() == 1 => KeyCode::Char(k.chars().next().unwrap()),
                        _ => continue,
                    };
                    self.key(code);
                }
            } else if let Some(t) = s.strip_prefix("type:") {
                self.typ(t);
            } else if let Some(xy) = s.strip_prefix("click:") {
                let (x, y) = xy.split_once(',').unwrap();
                self.click(x.parse().unwrap(), y.parse().unwrap());
            } else if let Some(xy) = s.strip_prefix("rclick:") {
                let (x, y) = xy.split_once(',').unwrap();
                self.rclick(x.parse().unwrap(), y.parse().unwrap());
            } else if let Some(t) = s.strip_prefix("text:") {
                self.click_text(t);
            }
        }
    }
}
