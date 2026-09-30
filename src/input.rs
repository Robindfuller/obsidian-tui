//! A one-line text box: the filter and search field, with the usual
//! readline keys (ctrl+a/e/w/u/k, ctrl+left/right by word).

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::util::char_w;

pub enum Ed {
    Changed,
    Moved,
    Submit,
    Ignored,
}

#[derive(Default, Clone)]
pub struct LineEdit {
    pub chars: Vec<char>,
    pub cur: usize,
    pub scroll: usize,
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

impl LineEdit {
    pub fn new(s: &str) -> LineEdit {
        let chars: Vec<char> = s.chars().collect();
        let cur = chars.len();
        LineEdit { chars, cur, scroll: 0 }
    }

    pub fn value(&self) -> String {
        self.chars.iter().collect()
    }

    pub fn set(&mut self, s: &str) {
        self.chars = s.chars().collect();
        self.cur = self.chars.len();
        self.scroll = 0;
    }

    pub fn insert(&mut self, s: &str) {
        for c in s.chars() {
            if c == '\n' || c == '\r' {
                continue;
            }
            self.chars.insert(self.cur, c);
            self.cur += 1;
        }
    }

    fn word_left(&self) -> usize {
        let mut i = self.cur;
        while i > 0 && !is_word(self.chars[i - 1]) {
            i -= 1;
        }
        while i > 0 && is_word(self.chars[i - 1]) {
            i -= 1;
        }
        i
    }

    fn word_right(&self) -> usize {
        let mut i = self.cur;
        let n = self.chars.len();
        while i < n && !is_word(self.chars[i]) {
            i += 1;
        }
        while i < n && is_word(self.chars[i]) {
            i += 1;
        }
        i
    }

    pub fn key(&mut self, k: &KeyEvent) -> Ed {
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        let alt = k.modifiers.contains(KeyModifiers::ALT);
        match k.code {
            KeyCode::Enter => Ed::Submit,
            KeyCode::Char('a') if ctrl => {
                self.cur = 0;
                Ed::Moved
            }
            KeyCode::Char('e') if ctrl => {
                self.cur = self.chars.len();
                Ed::Moved
            }
            KeyCode::Char('d') if ctrl => self.delete_right(),
            KeyCode::Char('w') if ctrl => {
                let to = self.word_left();
                self.chars.drain(to..self.cur);
                self.cur = to;
                Ed::Changed
            }
            KeyCode::Char('u') if ctrl => {
                self.chars.drain(..self.cur);
                self.cur = 0;
                Ed::Changed
            }
            KeyCode::Char('k') if ctrl => {
                self.chars.truncate(self.cur);
                Ed::Changed
            }
            KeyCode::Char(c) if !ctrl && !alt => {
                self.chars.insert(self.cur, c);
                self.cur += 1;
                Ed::Changed
            }
            KeyCode::Backspace if ctrl || alt => {
                let to = self.word_right();
                self.chars.drain(self.cur..to);
                Ed::Changed
            }
            KeyCode::Backspace => {
                if self.cur > 0 {
                    self.cur -= 1;
                    self.chars.remove(self.cur);
                    Ed::Changed
                } else {
                    Ed::Moved
                }
            }
            KeyCode::Delete => self.delete_right(),
            KeyCode::Left if ctrl => {
                self.cur = self.word_left();
                Ed::Moved
            }
            KeyCode::Right if ctrl => {
                self.cur = self.word_right();
                Ed::Moved
            }
            KeyCode::Left => {
                self.cur = self.cur.saturating_sub(1);
                Ed::Moved
            }
            KeyCode::Right => {
                self.cur = (self.cur + 1).min(self.chars.len());
                Ed::Moved
            }
            KeyCode::Home => {
                self.cur = 0;
                Ed::Moved
            }
            KeyCode::End => {
                self.cur = self.chars.len();
                Ed::Moved
            }
            _ => Ed::Ignored,
        }
    }

    fn delete_right(&mut self) -> Ed {
        if self.cur < self.chars.len() {
            self.chars.remove(self.cur);
            Ed::Changed
        } else {
            Ed::Moved
        }
    }

    /// The visible slice for a box `w` cells wide, and where the cursor sits in it.
    pub fn view(&mut self, w: usize) -> (String, usize) {
        let w = w.max(1);
        if self.cur < self.scroll {
            self.scroll = self.cur;
        }
        // keep the cursor cell inside the box
        loop {
            let used: usize = self.chars[self.scroll..self.cur].iter().map(|c| char_w(*c)).sum();
            if used < w || self.scroll >= self.cur {
                break;
            }
            self.scroll += 1;
        }
        let mut out = String::new();
        let mut used = 0;
        let mut cur_x = 0;
        for (i, c) in self.chars.iter().enumerate().skip(self.scroll) {
            if i == self.cur {
                cur_x = used;
            }
            let cw = char_w(*c);
            if used + cw > w {
                break;
            }
            out.push(*c);
            used += cw;
        }
        if self.cur >= self.chars.len() {
            cur_x = used;
        }
        (out, cur_x)
    }
}
