//! Styled, clickable text. A `Seg` is a run of text with a colour and, maybe,
//! the action a click on it runs. `wrap` folds segments into lines: words move
//! down whole, a word too long for the line is cut.

use ratatui::style::Color;

use crate::util::{cell_len, char_w};

/// Everything a click (or a key standing in for one) can do.
#[derive(Clone, Debug, PartialEq)]
pub enum Act {
    ToggleRail,
    /// a sidebar entry: "all", "recent", "search", "folder:<key>", "tag:<tag>"
    Rail(String),
    /// open or close a folder in the tree
    Fold(String),
    /// open a note, maybe at a heading
    Open { key: String, heading: Option<String> },
    /// a link to a note that doesn't exist
    Missing(String),
    /// a web link or other URL
    Url(String),
    /// an attachment in the vault (image, PDF...)
    File(String),
    Tag(String),
    Tab(usize),
    /// jump to a line of the open note (the outline)
    Line(usize),
    Back,
    /// a narrow window's step back
    StageBack,
    Forward,
    Edit,
    Obsidian,
    CopyPath,
    CopyLink,
    Filter,
    Search,
    ClearSearch,
    Sort,
    NextHit,
    PrevHit,
    Help,
    Quit,
    MenuPick(usize),
    Toast(u64),
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct Seg {
    pub text: String,
    pub fg: Option<Color>,
    pub bg: Option<Color>,
    pub bold: bool,
    pub italic: bool,
    pub strike: bool,
    pub under: bool,
    pub act: Option<Act>,
}

pub fn sp(text: impl Into<String>, fg: Color) -> Seg {
    Seg { text: text.into(), fg: Some(fg), ..Seg::default() }
}

/// Text in whatever colour is around it.
pub fn plain(text: impl Into<String>) -> Seg {
    Seg { text: text.into(), ..Seg::default() }
}

impl Seg {
    pub fn b(mut self) -> Seg {
        self.bold = true;
        self
    }
    pub fn b_if(mut self, on: bool) -> Seg {
        self.bold = on;
        self
    }
    pub fn i(mut self) -> Seg {
        self.italic = true;
        self
    }
    pub fn s(mut self) -> Seg {
        self.strike = true;
        self
    }
    pub fn u(mut self) -> Seg {
        self.under = true;
        self
    }
    pub fn bg(mut self, c: Color) -> Seg {
        self.bg = Some(c);
        self
    }
    pub fn on(mut self, a: Act) -> Seg {
        self.act = Some(a);
        self
    }
}

pub type Line = Vec<Seg>;

pub fn line_width(l: &[Seg]) -> usize {
    l.iter().map(|s| cell_len(&s.text)).sum()
}

/// Join runs into one list, with "\n" as line breaks between them.
pub fn join(parts: Vec<Vec<Seg>>, sep: &str) -> Vec<Seg> {
    let mut out = vec![];
    for (i, p) in parts.into_iter().enumerate() {
        if i > 0 && !sep.is_empty() {
            out.push(plain(sep));
        }
        out.extend(p);
    }
    out
}

fn regroup(chars: &[(char, usize)], segs: &[Seg]) -> Line {
    let mut out: Line = vec![];
    let mut last = usize::MAX;
    for &(c, i) in chars {
        if i == last {
            out.last_mut().unwrap().text.push(c);
        } else {
            let mut s = segs[i].clone();
            s.text = c.to_string();
            out.push(s);
            last = i;
        }
    }
    out
}

/// Split on newlines and fold each line to `width` cells.
pub fn wrap(segs: &[Seg], width: usize) -> Vec<Line> {
    let width = width.max(1);
    let mut logical: Vec<Vec<(char, usize)>> = vec![vec![]];
    for (i, s) in segs.iter().enumerate() {
        for c in s.text.chars() {
            if c == '\n' {
                logical.push(vec![]);
            } else {
                logical.last_mut().unwrap().push((c, i));
            }
        }
    }
    let mut out = vec![];
    for chars in logical {
        // words: leading space, the word, trailing space (Rich's \s*\S+\s*)
        let mut words: Vec<Vec<(char, usize)>> = vec![];
        let mut cur: Vec<(char, usize)> = vec![];
        let mut in_trail = false;
        for &(c, i) in &chars {
            let space = c == ' ' || c == '\t';
            if !space && in_trail {
                words.push(std::mem::take(&mut cur));
                in_trail = false;
            }
            if space && cur.iter().any(|(x, _)| *x != ' ' && *x != '\t') {
                in_trail = true;
            }
            cur.push((c, i));
        }
        if !cur.is_empty() {
            words.push(cur);
        }
        let mut lines: Vec<Vec<(char, usize)>> = vec![vec![]];
        let mut pos = 0usize;
        for w in words {
            let trimmed_len = {
                let mut end = w.len();
                while end > 0 && (w[end - 1].0 == ' ' || w[end - 1].0 == '\t') {
                    end -= 1;
                }
                w[..end].iter().map(|(c, _)| char_w(*c)).sum::<usize>()
            };
            let full: usize = w.iter().map(|(c, _)| char_w(*c)).sum();
            if pos + trimmed_len > width {
                if trimmed_len > width {
                    // fold the long word across lines
                    for &(c, i) in &w {
                        let cw = char_w(c);
                        if pos + cw > width && pos > 0 {
                            lines.push(vec![]);
                            pos = 0;
                        }
                        lines.last_mut().unwrap().push((c, i));
                        pos += cw;
                    }
                    continue;
                }
                if pos > 0 {
                    lines.push(vec![]);
                }
                pos = 0;
            }
            lines.last_mut().unwrap().extend(w.iter().copied());
            pos += full;
        }
        for mut l in lines {
            while l.last().is_some_and(|(c, _)| *c == ' ' || *c == '\t') {
                l.pop();
            }
            out.push(regroup(&l, segs));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(l: &Line) -> String {
        l.iter().map(|s| s.text.as_str()).collect()
    }

    #[test]
    fn wraps_words() {
        let segs = vec![plain("the quick brown fox jumps")];
        let lines: Vec<String> = wrap(&segs, 10).iter().map(text).collect();
        assert_eq!(lines, vec!["the quick", "brown fox", "jumps"]);
    }

    #[test]
    fn keeps_newlines_and_folds_long_words() {
        let segs = vec![plain("ab\n\nabcdefghij")];
        let lines: Vec<String> = wrap(&segs, 4).iter().map(text).collect();
        assert_eq!(lines, vec!["ab", "", "abcd", "efgh", "ij"]);
    }
}
