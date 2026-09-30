//! Ask the vault: a question goes to a language model along with the notes
//! that match it best, and the answer streams back naming the notes it used.
//! The model is local (Ollama) or Claude through the Claude Code CLI, so it
//! runs on your own Claude plan. Nothing here writes to the vault.
//!
//! Finding the notes is plain word matching, weighted so rare words count for
//! more (no embeddings, nothing to index). From each note it sends the whole
//! text if it's short, else the paragraphs that mention the question's words.

use std::collections::HashSet;
use std::io::{BufRead, BufReader, Write};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::input::LineEdit;
use crate::md::Rendered;
use crate::vault::{Note, Vault};

/// The ask pane's settings, changed in the Settings panel and kept in the
/// state file under "ask".
#[derive(Clone, Debug, PartialEq)]
pub struct Conf {
    /// where Ollama is; empty means OLLAMA_HOST, else this computer
    pub host: String,
    /// notes sent with a question, at most
    pub notes: usize,
    /// note text sent, in characters: to a local model, and to Claude
    pub local_chars: usize,
    pub claude_chars: usize,
    /// Ollama's context window, in tokens
    pub ctx: usize,
    /// let models that can reason think first
    pub think: bool,
    /// earlier questions and answers sent with a follow-up
    pub turns: usize,
    /// added to the model's instructions
    pub extra: String,
}

impl Default for Conf {
    fn default() -> Conf {
        Conf {
            host: String::new(),
            notes: 6,
            local_chars: 12_000,
            claude_chars: 40_000,
            ctx: 8192,
            think: false,
            turns: 4,
            extra: String::new(),
        }
    }
}

impl Conf {
    /// From the state file, anything missing or odd left at its default.
    pub fn from_json(v: &Value) -> Conf {
        let d = Conf::default();
        let num = |k: &str, def: usize, lo: usize, hi: usize| v[k].as_u64().map(|x| (x as usize).clamp(lo, hi)).unwrap_or(def);
        Conf {
            host: v["host"].as_str().unwrap_or("").trim().to_string(),
            notes: num("notes", d.notes, 1, 20),
            local_chars: num("local_chars", d.local_chars, 1000, 200_000),
            claude_chars: num("claude_chars", d.claude_chars, 1000, 400_000),
            ctx: num("ctx", d.ctx, 1024, 262_144),
            think: v["think"].as_bool().unwrap_or(d.think),
            turns: num("turns", d.turns, 0, 20),
            extra: v["extra"].as_str().unwrap_or("").to_string(),
        }
    }

    pub fn to_json(&self) -> Value {
        json!({
            "host": self.host,
            "notes": self.notes,
            "local_chars": self.local_chars,
            "claude_chars": self.claude_chars,
            "ctx": self.ctx,
            "think": self.think,
            "turns": self.turns,
            "extra": self.extra,
        })
    }

    /// Where to reach Ollama.
    pub fn ollama(&self) -> String {
        ollama_host(&self.host)
    }
}

/// Words that say nothing about which note is meant.
const STOP: &[&str] = &[
    "a", "about", "above", "after", "again", "all", "also", "am", "an", "and", "any", "anything", "are", "as", "at",
    "be", "been", "before", "being", "but", "by", "can", "could", "did", "do", "does", "doing", "done", "down", "each",
    "else", "ever", "every", "find", "for", "from", "get", "give", "got", "had", "has", "have", "having", "he", "her",
    "here", "him", "his", "how", "i", "if", "in", "into", "is", "it", "its", "just", "know", "let", "like", "list",
    "many", "me", "mention", "mentioned", "more", "most", "much", "my", "no", "not", "note", "notes", "now", "of",
    "off", "on", "once", "only", "or", "other", "our", "out", "over", "please", "said", "say", "says", "she", "should",
    "show", "so", "some", "something", "tell", "than", "that", "the", "their", "them", "then", "there", "these",
    "they", "thing", "things", "this", "those", "to", "too", "up", "us", "vault", "very", "was", "we", "were", "what",
    "whats", "when", "where", "which", "while", "who", "why", "will", "with", "would", "write", "wrote", "you", "your",
];

/// Words that mean "the note on screen".
const HERE: &[&str] = &["this", "here", "current", "open", "above"];

/// The question's words that could pick out a note: lowercased, common words
/// dropped, plurals and -ing/-ed trimmed so "beds" finds "bed".
pub fn words(q: &str) -> Vec<String> {
    let mut out: Vec<String> = vec![];
    let lower = q.to_lowercase().replace(['\'', '’'], "");
    for w in lower.split(|c: char| !c.is_alphanumeric()) {
        if w.is_empty() || STOP.contains(&w) || (w.chars().count() < 3 && !w.chars().all(|c| c.is_ascii_digit())) {
            continue;
        }
        let w = stem(w);
        if !out.contains(&w) {
            out.push(w);
        }
    }
    out
}

fn stem(w: &str) -> String {
    let n = w.chars().count();
    for (suf, min) in [("ies", 5), ("ing", 6), ("ed", 5)] {
        if n >= min
            && let Some(s) = w.strip_suffix(suf)
        {
            return if suf == "ies" { format!("{s}y") } else { s.to_string() };
        }
    }
    if n >= 4 && w.ends_with('s') && !w.ends_with("ss") && !w.ends_with("us") {
        return w[..w.len() - 1].to_string();
    }
    w.to_string()
}

/// A note sent with the question, and the text of it that was sent.
#[derive(Clone, Debug)]
pub struct Pick {
    pub key: String,
    pub text: String,
    /// the note that was open on screen
    pub open: bool,
}

/// The notes that best match `q`, best first, with the parts worth sending.
/// `open` is the note on screen: it's added when the question points at it
/// ("this note") or nothing else matched. `before` are the last answer's
/// notes, kept for a follow-up that names nothing new.
pub fn gather(v: &Vault, q: &str, open: Option<&str>, before: &[String], max: usize, budget: usize) -> Vec<Pick> {
    let ws = words(q);
    // BM25: rarer words weigh more, and a word counts for less in a long note
    let n = v.notes.len().max(1) as f64;
    let avg = v.notes.iter().map(|x| x.lower.len()).sum::<usize>() as f64 / n;
    let idf: Vec<f64> = ws
        .iter()
        .map(|w| {
            let df = v.notes.iter().filter(|x| x.lower.contains(w.as_str())).count() as f64;
            if df == 0.0 { 0.0 } else { (1.0 + (n - df + 0.5) / (df + 0.5)).ln() }
        })
        .collect();
    let (k1, b) = (1.2, 0.75);
    let mut scored: Vec<(f64, usize)> = vec![];
    for (i, note) in v.notes.iter().enumerate() {
        let name = note.name.to_lowercase();
        let name_words: Vec<String> = name.split(|c: char| !c.is_alphanumeric()).map(stem).collect();
        let len = note.lower.len() as f64 / avg.max(1.0);
        let mut s = 0.0;
        let mut hit = 0;
        for (w, idf) in ws.iter().zip(&idf) {
            let tf = note.lower.matches(w.as_str()).count() as f64;
            if tf > 0.0 {
                hit += 1;
                s += idf * tf * (k1 + 1.0) / (tf + k1 * (1.0 - b + b * len));
            }
            // the title says it: the note is about it
            if name_words.iter().any(|x| x == w) {
                s += 2.5 * idf;
            } else if name.contains(w.as_str()) {
                s += idf;
            }
            if note.tags.iter().any(|t| t.to_lowercase().contains(w.as_str())) {
                s += 0.5 * idf;
            }
        }
        if s > 0.0 {
            // a note with more of the words beats one that says one word a lot
            s *= 0.5 + hit as f64 / ws.len() as f64;
            scored.push((s, i));
        }
    }
    scored.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)));
    let best = scored.first().map(|s| s.0).unwrap_or(0.0);
    let mut keys: Vec<String> = scored
        .iter()
        .take_while(|(s, _)| *s >= best * 0.2)
        .take(max)
        .map(|(_, i)| v.notes[*i].key.clone())
        .collect();
    if keys.len() < 2 {
        for k in before {
            if keys.len() < max && !keys.contains(k) && v.note(k).is_some() {
                keys.push(k.clone());
            }
        }
    }
    let lower = q.to_lowercase();
    let here = lower.split(|c: char| !c.is_alphanumeric()).any(|w| HERE.contains(&w));
    let mut open_key = None;
    if let Some(o) = open
        && v.note(o).is_some()
        && (here || keys.is_empty())
    {
        keys.retain(|k| k != o);
        keys.insert(0, o.to_string());
        open_key = Some(o.to_string());
    }
    // share the room out: the first notes get more
    let mut out = vec![];
    let mut left = budget;
    for (i, k) in keys.iter().enumerate() {
        let Some(note) = v.note(k) else { continue };
        let share = if i == 0 { left / 2 } else { left / (keys.len() - i + 1).max(2) };
        let share = share.max(600).min(left);
        if share < 200 {
            break;
        }
        let text = passages(note, &ws, share);
        left = left.saturating_sub(text.len());
        out.push(Pick { key: k.clone(), text, open: open_key.as_deref() == Some(k.as_str()) });
    }
    out
}

/// A note's text, or the paragraphs of it that mention the words (with the
/// heading each sits under), in the note's order, up to `max` bytes.
pub fn passages(note: &Note, ws: &[String], max: usize) -> String {
    let body = note.body().trim();
    if body.len() <= max {
        return body.to_string();
    }
    // paragraphs, each with the heading above it
    let mut blocks: Vec<(String, String)> = vec![];
    let mut heading = String::new();
    let mut cur = String::new();
    let flush = |cur: &mut String, blocks: &mut Vec<(String, String)>, heading: &str| {
        if !cur.trim().is_empty() {
            blocks.push((heading.to_string(), cur.trim().to_string()));
        }
        cur.clear();
    };
    for line in body.lines() {
        if line.starts_with('#') && line.trim_start_matches('#').starts_with(' ') {
            flush(&mut cur, &mut blocks, &heading);
            heading = line.trim().to_string();
            continue;
        }
        if line.trim().is_empty() {
            flush(&mut cur, &mut blocks, &heading);
            continue;
        }
        cur.push_str(line);
        cur.push('\n');
    }
    flush(&mut cur, &mut blocks, &heading);
    let mut ranked: Vec<(usize, usize)> = blocks
        .iter()
        .enumerate()
        .map(|(i, (h, b))| {
            let t = format!("{h}\n{b}").to_lowercase();
            (ws.iter().map(|w| t.matches(w.as_str()).count()).sum::<usize>(), i)
        })
        .collect();
    // the opening paragraph always, then the ones that say the most
    ranked.sort_by(|a, b| (b.1 == 0).cmp(&(a.1 == 0)).then(b.0.cmp(&a.0)).then(a.1.cmp(&b.1)));
    let mut chosen: Vec<usize> = vec![];
    let mut used = 0;
    for (score, i) in ranked {
        if score == 0 && i != 0 {
            break;
        }
        let len = blocks[i].1.len() + blocks[i].0.len() + 2;
        if used + len > max {
            if used == 0 {
                // one huge paragraph: cut it
                chosen.push(i);
            }
            continue;
        }
        used += len;
        chosen.push(i);
    }
    chosen.sort();
    let mut out = String::new();
    let mut last_heading = String::new();
    let mut prev: Option<usize> = None;
    for i in chosen {
        let (h, b) = &blocks[i];
        if prev.is_some_and(|p| p + 1 != i) {
            out.push_str("…\n\n");
        }
        if !h.is_empty() && *h != last_heading {
            out.push_str(h);
            out.push_str("\n\n");
            last_heading = h.clone();
        }
        out.push_str(b);
        out.push_str("\n\n");
        prev = Some(i);
    }
    if out.len() > max {
        let mut cut = max;
        while !out.is_char_boundary(cut) {
            cut -= 1;
        }
        out.truncate(cut);
        out.push('…');
    }
    out.trim_end().to_string()
}

// ------------------------------------------------------------ models

/// Something that can answer: "ollama:<model>", "claude:<model>", or "fake"
/// (the tests' stand-in).
#[derive(Clone, Debug, PartialEq)]
pub struct Model {
    pub id: String,
    pub label: String,
    pub hint: String,
}

impl Model {
    pub fn local(&self) -> bool {
        !self.id.starts_with("claude:")
    }
}

/// Where Ollama is: the setting, else OLLAMA_HOST, else this computer.
pub fn ollama_host(set: &str) -> String {
    let h = if set.trim().is_empty() { std::env::var("OLLAMA_HOST").unwrap_or_default() } else { set.to_string() };
    let h = h.trim().trim_end_matches('/');
    if h.is_empty() {
        return "http://localhost:11434".into();
    }
    let h = if h.contains("://") { h.to_string() } else { format!("http://{h}") };
    // Ollama listens on 0.0.0.0 but that isn't somewhere to connect to
    let h = h.replace("://0.0.0.0", "://localhost");
    let after = h.split_once("://").map(|x| x.1).unwrap_or(&h);
    if after.contains(':') { h } else { format!("{h}:11434") }
}

fn agent(connect: Duration, total: Option<Duration>) -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_connect(Some(connect))
        .timeout_global(total)
        .http_status_as_error(false)
        .build()
        .into()
}

/// The chat models Ollama has, biggest first.
pub fn ollama_models(host: &str) -> Result<Vec<Model>, String> {
    let url = format!("{host}/api/tags");
    let mut r = agent(Duration::from_millis(800), Some(Duration::from_secs(3)))
        .get(&url)
        .call()
        .map_err(|e| e.to_string())?;
    let body = r.body_mut().read_to_string().map_err(|e| e.to_string())?;
    let v: Value = serde_json::from_str(&body).map_err(|e| e.to_string())?;
    let mut ms: Vec<(u64, Model)> = v["models"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|m| match m["capabilities"].as_array() {
            Some(c) => c.iter().any(|x| x == "completion"),
            None => !m["name"].as_str().unwrap_or("").contains("embed"),
        })
        .filter_map(|m| {
            let name = m["name"].as_str()?.to_string();
            let size = m["size"].as_u64().unwrap_or(0);
            let hint = format!("on this computer · {:.1} GB", size as f64 / 1e9);
            Some((size, Model { id: format!("ollama:{name}"), label: name, hint }))
        })
        .collect();
    ms.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.label.cmp(&b.1.label)));
    Ok(ms.into_iter().map(|x| x.1).collect())
}

/// Is the Claude Code CLI on the PATH?
pub fn have_claude() -> bool {
    let Some(path) = std::env::var_os("PATH") else { return false };
    std::env::split_paths(&path).any(|d| {
        ["claude", "claude.exe", "claude.cmd"].iter().any(|n| d.join(n).is_file())
    })
}

pub fn claude_models() -> Vec<Model> {
    let hint = "Claude · uses your Claude plan".to_string();
    vec![
        Model { id: "claude:haiku".into(), label: "Claude Haiku".into(), hint: hint.clone() },
        Model { id: "claude:sonnet".into(), label: "Claude Sonnet".into(), hint: hint.clone() },
        Model { id: "claude:opus".into(), label: "Claude Opus".into(), hint },
    ]
}

/// Everything there is to ask, and why Ollama isn't in the list if it isn't.
pub fn all_models(host: &str) -> (Vec<Model>, Option<String>) {
    let (mut out, why) = match ollama_models(host) {
        Ok(m) if m.is_empty() => (vec![], Some("Ollama has no models yet. Try: ollama pull llama3.2".to_string())),
        Ok(m) => (m, None),
        Err(_) => (vec![], Some(format!("Ollama isn't running at {host}."))),
    };
    if have_claude() {
        out.extend(claude_models());
    }
    (out, why)
}

pub fn fake_model() -> Model {
    Model { id: "fake".into(), label: "test model".into(), hint: "canned answers".into() }
}

// ------------------------------------------------------------ asking

/// What the model sends back, a piece at a time.
pub enum Msg {
    Text(String),
    /// a reasoning model is thinking before it answers
    Thinking,
    Done,
    Err(String),
}

/// One question and its answer.
pub struct Turn {
    pub q: String,
    pub a: String,
    /// the notes sent with the question
    pub sources: Vec<String>,
    pub err: Option<String>,
    pub done: bool,
    /// the model's thinking, before any answer
    pub thinking: bool,
    pub at: Instant,
    /// the answer drawn: text length, width, theme generation
    pub cache: Option<(usize, usize, u64, std::rc::Rc<Rendered>)>,
}

impl Turn {
    /// The answer without any <think> block a reasoning model put first.
    pub fn answer(&self) -> String {
        let a = self.a.trim_start();
        if let Some(rest) = a.strip_prefix("<think>") {
            return match rest.split_once("</think>") {
                Some((_, after)) => after.trim_start().to_string(),
                None => String::new(),
            };
        }
        a.to_string()
    }
}

#[derive(Default)]
pub struct Chat {
    pub turns: Vec<Turn>,
    pub input: LineEdit,
    /// the top line shown; None keeps the end in view
    pub scroll: Option<usize>,
    /// lines in the transcript and rows for it, at the last draw
    pub lines: usize,
    pub rows: usize,
    rx: Option<Receiver<Msg>>,
    stop: Option<Arc<AtomicBool>>,
}

impl Chat {
    pub fn busy(&self) -> bool {
        self.rx.is_some()
    }

    pub fn stop(&mut self) {
        if let Some(s) = self.stop.take() {
            s.store(true, Ordering::Relaxed);
        }
        self.rx = None;
        if let Some(t) = self.turns.last_mut()
            && !t.done
        {
            t.done = true;
            if t.a.trim().is_empty() {
                t.err = Some("Stopped.".into());
            }
        }
    }

    /// Take in what's arrived. True if anything changed.
    pub fn poll(&mut self) -> bool {
        let Some(rx) = &self.rx else { return false };
        let mut changed = false;
        let mut end = false;
        loop {
            match rx.try_recv() {
                Ok(Msg::Thinking) => {
                    if let Some(t) = self.turns.last_mut()
                        && !t.thinking
                    {
                        t.thinking = true;
                        changed = true;
                    }
                }
                Ok(Msg::Text(s)) => {
                    if let Some(t) = self.turns.last_mut() {
                        t.a.push_str(&s);
                    }
                    changed = true;
                }
                Ok(Msg::Done) => {
                    end = true;
                    break;
                }
                Ok(Msg::Err(e)) => {
                    if let Some(t) = self.turns.last_mut() {
                        t.err = Some(e);
                    }
                    end = true;
                    break;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    end = true;
                    break;
                }
            }
        }
        if end {
            self.rx = None;
            self.stop = None;
            if let Some(t) = self.turns.last_mut() {
                t.done = true;
                if t.err.is_none() && t.answer().trim().is_empty() {
                    t.err = Some("The model didn't say anything.".into());
                }
            }
            changed = true;
        }
        changed
    }

    /// Send the question in the box, with the notes that match it.
    pub fn ask(&mut self, v: &Vault, open: Option<&str>, model: &Model, conf: &Conf) {
        let q = self.input.value().trim().to_string();
        if q.is_empty() || self.busy() {
            return;
        }
        self.input.set("");
        let before = self.turns.last().map(|t| t.sources.clone()).unwrap_or_default();
        // a local model gets less to read: it's slower, and its window smaller
        let budget = if model.local() { conf.local_chars } else { conf.claude_chars };
        let picks = gather(v, &q, open, &before, conf.notes, budget);
        let (system, messages) = prompt(v, &self.turns, &q, &picks, conf);
        self.turns.push(Turn {
            q,
            a: String::new(),
            sources: picks.iter().map(|p| p.key.clone()).collect(),
            err: None,
            done: false,
            thinking: false,
            at: Instant::now(),
            cache: None,
        });
        self.scroll = None;
        let (tx, rx) = channel();
        let stop = Arc::new(AtomicBool::new(false));
        self.rx = Some(rx);
        self.stop = Some(stop.clone());
        let id = model.id.clone();
        let conf = conf.clone();
        if id == "fake" {
            fake(&picks, &tx);
            return;
        }
        std::thread::spawn(move || {
            let r = if let Some(m) = id.strip_prefix("ollama:") {
                ollama(m, &system, &messages, &conf, &tx, &stop)
            } else if let Some(m) = id.strip_prefix("claude:") {
                claude(m, &system, &messages, &tx, &stop)
            } else {
                Err(format!("Don't know how to ask {id}."))
            };
            let _ = tx.send(match r {
                Ok(()) => Msg::Done,
                Err(e) => Msg::Err(e),
            });
        });
    }
}

/// Today's date as YYYY-MM-DD (days to civil date, Howard Hinnant's way).
fn today() -> String {
    let secs = crate::util::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0) as i64;
    let z = secs.div_euclid(86_400) + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
    format!("{y:04}-{m:02}-{d:02}")
}

/// The instructions, and the chat so far with the notes on the new question.
pub fn prompt(v: &Vault, turns: &[Turn], q: &str, picks: &[Pick], conf: &Conf) -> (String, Vec<(String, String)>) {
    let mut system = format!(
        "You help someone find things in their own notes, an Obsidian vault called \"{}\". Today is {}.\n\
         Answer from the notes you're given, and nothing else. When you use a note, name it as a wiki link \
         with its exact title, like [[Garden Plan]]. If the notes don't answer the question, say so plainly; \
         never make things up. Keep it short and clear, in plain British English. Markdown is fine.",
        v.name,
        today()
    );
    if !conf.extra.trim().is_empty() {
        system.push_str("\n\n");
        system.push_str(conf.extra.trim());
    }
    let mut messages = vec![];
    // the last few turns, without the notes that went with them
    for t in turns.iter().rev().take(conf.turns).collect::<Vec<_>>().into_iter().rev() {
        if t.err.is_some() && t.answer().trim().is_empty() {
            continue;
        }
        messages.push(("user".to_string(), t.q.clone()));
        messages.push(("assistant".to_string(), t.answer()));
    }
    let mut msg = String::new();
    if picks.is_empty() {
        msg.push_str("(No notes matched this question.)\n\n");
    } else {
        msg.push_str("Notes from the vault that might help:\n\n");
        for p in picks {
            let Some(n) = v.note(&p.key) else { continue };
            let open = if p.open { " open-on-screen=\"yes\"" } else { "" };
            msg.push_str(&format!("<note title=\"{}\" folder=\"{}\"{open}>\n", n.name, n.folder));
            msg.push_str(&p.text);
            msg.push_str("\n</note>\n\n");
        }
    }
    msg.push_str("Question: ");
    msg.push_str(q);
    messages.push(("user".to_string(), msg));
    (system, messages)
}

fn fake(picks: &[Pick], tx: &Sender<Msg>) {
    let answer = match picks.first() {
        Some(p) => {
            let name = p.key.rsplit('/').next().unwrap_or(&p.key).trim_end_matches(".md").to_string();
            format!("It's in [[{name}]]: **{} note{}** looked at.", picks.len(), if picks.len() == 1 { "" } else { "s" })
        }
        None => "Nothing in the notes says.".to_string(),
    };
    for w in answer.split_inclusive(' ') {
        let _ = tx.send(Msg::Text(w.to_string()));
    }
    let _ = tx.send(Msg::Done);
}

fn ollama(model: &str, system: &str, messages: &[(String, String)], conf: &Conf, tx: &Sender<Msg>, stop: &AtomicBool) -> Result<(), String> {
    match ollama_chat(model, system, messages, conf, conf.think, tx, stop) {
        // thinking asked of a model that can't: ask again without
        Err(e) if conf.think && e.contains("does not support thinking") => {
            ollama_chat(model, system, messages, conf, false, tx, stop)
        }
        r => r,
    }
}

#[allow(clippy::too_many_arguments)]
fn ollama_chat(
    model: &str,
    system: &str,
    messages: &[(String, String)],
    conf: &Conf,
    think: bool,
    tx: &Sender<Msg>,
    stop: &AtomicBool,
) -> Result<(), String> {
    let host = conf.ollama();
    let mut msgs = vec![json!({"role": "system", "content": system})];
    msgs.extend(messages.iter().map(|(r, c)| json!({"role": r, "content": c})));
    let body = json!({
        "model": model,
        "messages": msgs,
        "stream": true,
        // straight to the answer unless asked: a reasoning model's thinking is slow
        "think": think,
        "options": {"num_ctx": conf.ctx},
    });
    let r = agent(Duration::from_secs(3), None)
        .post(&format!("{host}/api/chat"))
        .header("Content-Type", "application/json")
        .send(body.to_string());
    let mut r = match r {
        Ok(r) => r,
        Err(ureq::Error::ConnectionFailed | ureq::Error::Io(_) | ureq::Error::HostNotFound) => {
            return Err(format!("Couldn't reach Ollama at {host}. Is it running? (ollama serve)"));
        }
        Err(e) => return Err(format!("Ollama: {e}")),
    };
    if !r.status().is_success() {
        let text = r.body_mut().read_to_string().unwrap_or_default();
        let msg = serde_json::from_str::<Value>(&text).ok().and_then(|v| v["error"].as_str().map(str::to_string));
        return Err(format!("Ollama said: {}", msg.unwrap_or(text)));
    }
    let reader = BufReader::new(r.into_body().into_reader());
    for line in reader.lines() {
        if stop.load(Ordering::Relaxed) {
            // dropping the connection stops Ollama too
            return Ok(());
        }
        let line = line.map_err(|e| format!("Lost Ollama: {e}"))?;
        let Ok(v) = serde_json::from_str::<Value>(&line) else { continue };
        if let Some(e) = v["error"].as_str() {
            return Err(format!("Ollama said: {e}"));
        }
        if v["message"]["thinking"].as_str().is_some_and(|s| !s.is_empty()) {
            let _ = tx.send(Msg::Thinking);
        }
        if let Some(s) = v["message"]["content"].as_str()
            && !s.is_empty()
            && tx.send(Msg::Text(s.to_string())).is_err()
        {
            return Ok(());
        }
        if v["done"].as_bool() == Some(true) {
            break;
        }
    }
    Ok(())
}

/// Claude through the Claude Code CLI: `claude -p`, so it's billed to the
/// plan it's logged in with. No tools, no session kept.
fn claude(model: &str, system: &str, messages: &[(String, String)], tx: &Sender<Msg>, stop: &AtomicBool) -> Result<(), String> {
    use std::process::{Command, Stdio};
    // the CLI takes one prompt: the chat so far goes in as text
    let mut text = String::new();
    let (last, earlier) = messages.split_last().ok_or("nothing to ask")?;
    if !earlier.is_empty() {
        text.push_str("Earlier in this conversation:\n\n");
        for (r, c) in earlier {
            text.push_str(if r == "user" { "Q: " } else { "A: " });
            text.push_str(c);
            text.push_str("\n\n");
        }
        text.push_str("---\n\n");
    }
    text.push_str(&last.1);
    let mut child = Command::new("claude")
        .args(["-p", "--model", model, "--tools", "", "--output-format", "stream-json", "--verbose"])
        .args(["--include-partial-messages", "--no-session-persistence", "--strict-mcp-config"])
        .arg("--system-prompt")
        .arg(system)
        // not the vault: it shouldn't pick up a CLAUDE.md or anything else there
        .current_dir(std::env::temp_dir())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Couldn't run claude ({e}). Is Claude Code installed?"))?;
    if let Some(mut i) = child.stdin.take() {
        let _ = i.write_all(text.as_bytes());
    }
    let out = child.stdout.take().ok_or("no output from claude")?;
    let mut said = false;
    let mut err = None;
    for line in BufReader::new(out).lines() {
        if stop.load(Ordering::Relaxed) {
            let _ = child.kill();
            let _ = child.wait();
            return Ok(());
        }
        let Ok(line) = line else { break };
        let Ok(v) = serde_json::from_str::<Value>(&line) else { continue };
        match v["type"].as_str() {
            Some("stream_event") => {
                let e = &v["event"];
                if e["type"] == "content_block_delta" && e["delta"]["type"] == "text_delta"
                    && let Some(s) = e["delta"]["text"].as_str()
                {
                    said = true;
                    if tx.send(Msg::Text(s.to_string())).is_err() {
                        let _ = child.kill();
                        break;
                    }
                }
            }
            Some("result") => {
                if v["is_error"].as_bool() == Some(true) {
                    err = Some(v["result"].as_str().unwrap_or("Claude couldn't answer.").to_string());
                } else if !said && let Some(s) = v["result"].as_str() {
                    let _ = tx.send(Msg::Text(s.to_string()));
                }
            }
            _ => {}
        }
    }
    let status = child.wait().map_err(|e| e.to_string())?;
    if let Some(e) = err {
        return Err(format!("Claude: {e}"));
    }
    if !status.success() && !said {
        let mut msg = String::new();
        if let Some(mut s) = child.stderr.take() {
            let _ = std::io::Read::read_to_string(&mut s, &mut msg);
        }
        let msg = msg.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("it stopped with an error").to_string();
        return Err(format!("Claude: {msg}"));
    }
    Ok(())
}

/// Every note name in a list of keys, once.
pub fn names(v: &Vault, keys: &[String]) -> Vec<(String, String)> {
    let mut seen = HashSet::new();
    keys.iter()
        .filter_map(|k| v.note(k).map(|n| (k.clone(), n.name.clone())))
        .filter(|(k, _)| seen.insert(k.clone()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn words_drop_the_filler() {
        assert_eq!(words("What's the budget for the garden beds?"), vec!["budget", "garden", "bed"]);
        assert_eq!(words("who said anything about compost"), vec!["compost"]);
        assert_eq!(words("notes from 2026"), vec!["2026"]);
    }

    #[test]
    fn stems() {
        assert_eq!(stem("stories"), "story");
        assert_eq!(stem("planting"), "plant");
        assert_eq!(stem("glass"), "glass");
    }

    #[test]
    fn think_blocks_are_hidden() {
        let t = |a: &str| Turn { q: String::new(), a: a.into(), sources: vec![], err: None, done: false, thinking: false, at: Instant::now(), cache: None };
        assert_eq!(t("<think>hmm</think>\n\nYes.").answer(), "Yes.");
        assert_eq!(t("<think>still going").answer(), "");
        assert_eq!(t("Plain.").answer(), "Plain.");
    }
}
