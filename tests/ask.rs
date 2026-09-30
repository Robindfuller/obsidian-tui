//! The ask pane: opening and closing it, a question and its answer (from the
//! tests' stand-in model), the notes it looked at, and which notes get sent.

mod common;
use common::*;

use obsidian_tui::app::Stage;
use obsidian_tui::ask::{gather, words};
use obsidian_tui::vault::Vault;
use ratatui::crossterm::event::{KeyCode, KeyModifiers};

#[test]
fn a_opens_the_pane_and_esc_closes_it() {
    let mut t = h("ask-open", 150, 30);
    t.click_text("Garden Plan");
    let s = t.key(KeyCode::Char('a'));
    golden("ask-empty", &s);
    assert!(t.app.asking);
    assert!(s.contains("Ask your notes"));
    let s = t.key(KeyCode::Esc);
    assert!(!t.app.asking);
    assert!(s.contains("Four raised beds"), "the note is back:\n{s}");
}

#[test]
fn typing_goes_to_the_question() {
    let mut t = h("ask-typing", 150, 30);
    t.key(KeyCode::Char('a'));
    let s = t.typ("quit jumping");
    assert!(!t.app.quit, "q typed a letter, it didn't quit");
    assert_eq!(t.app.chat.input.value(), "quit jumping");
    assert!(s.contains("› quit jumping"));
}

#[test]
fn a_question_gets_an_answer_and_its_notes() {
    let mut t = h("ask-answer", 150, 30);
    let (v, _) = vault("ask-answer-snap");
    let before = snapshot(&v);
    t.key(KeyCode::Char('a'));
    t.typ("what is the budget for the garden beds?");
    t.key(KeyCode::Enter);
    let s = t.wait();
    golden("ask-answer", &s);
    assert!(s.contains("› what is the budget for the garden beds?"));
    // the answer's [[link]] is drawn as a link, and the notes it had are listed
    assert!(s.contains("It's in Garden Plan:"), "{s}");
    assert!(s.contains("Looked at  Garden Plan"), "{s}");
    assert_eq!(t.app.chat.input.value(), "", "the box is empty for the next question");
    // a note it looked at opens with a click, and the chat is kept
    let (x, y) = t.find("Looked at  Garden Plan").unwrap();
    t.click(x + 11, y);
    assert_eq!(note(&t), "Projects/Garden Plan.md");
    assert!(!t.app.asking);
    let s = t.key(KeyCode::Char('a'));
    assert!(s.contains("› what is the budget for the garden beds?"), "the chat is still there");
    assert_eq!(snapshot(&v), before, "asking never writes to the vault");
}

#[test]
fn the_link_in_an_answer_opens_the_note() {
    let mut t = h("ask-link", 150, 30);
    t.key(KeyCode::Char('a'));
    t.typ("the timber budget");
    t.key(KeyCode::Enter);
    t.wait();
    t.click_text("It's in Garden Plan");
    t.click_in_note("Garden Plan:");
    assert_eq!(note(&t), "Projects/Garden Plan.md");
}

#[test]
fn new_chat_and_the_model_menu() {
    let mut t = h("ask-new", 150, 30);
    t.key(KeyCode::Char('a'));
    t.typ("shelves");
    t.key(KeyCode::Enter);
    t.wait();
    assert_eq!(t.app.chat.turns.len(), 1);
    let s = t.click_text("test model ▾");
    assert!(s.contains("Answer with"));
    assert!(s.contains("● test model"));
    t.key(KeyCode::Esc);
    assert!(t.app.asking, "esc closed the menu, not the pane");
    let s = t.press(KeyCode::Char('o'), KeyModifiers::CONTROL);
    assert!(s.contains("ASK YOUR NOTES"), "ctrl+o is the settings");
    t.key(KeyCode::Esc);
    let s = t.click_text("new chat");
    assert!(t.app.chat.turns.is_empty());
    assert!(s.contains("Ask a question about the notes"));
}

#[test]
fn narrow_windows_show_the_pane_on_its_own() {
    let mut t = h("ask-narrow", 80, 24);
    let s = t.key(KeyCode::Char('a'));
    assert_eq!(t.app.shown(), Stage::Note);
    assert!(s.contains("‹ notes"));
    assert!(!s.contains("All notes"), "{s}");
    t.key(KeyCode::Esc);
    assert_eq!(t.app.shown(), Stage::List);
}

#[test]
fn a_long_answer_scrolls_and_follows_the_end() {
    let mut t = h("ask-scroll", 150, 16);
    t.key(KeyCode::Char('a'));
    for q in ["garden", "shelves", "books", "welcome"] {
        t.typ(q);
        t.key(KeyCode::Enter);
        t.wait();
    }
    let s = t.render();
    assert!(s.contains("› welcome"), "the newest is in view:\n{s}");
    for _ in 0..10 {
        t.key(KeyCode::PageUp);
    }
    assert_eq!(t.app.chat.scroll, Some(0));
    let s = t.render();
    assert!(s.contains("› garden") && !s.contains("› welcome"), "{s}");
    for _ in 0..10 {
        t.key(KeyCode::PageDown);
    }
    assert_eq!(t.app.chat.scroll, None, "back at the end it follows again");
}

// ------------------------------------------------------------ which notes go

fn fern(name: &str) -> Vault {
    let (v, _) = vault(name);
    Vault::open(&v)
}

fn keys(v: &Vault, q: &str, open: Option<&str>, before: &[String]) -> Vec<String> {
    gather(v, q, open, before, 6, 12_000).into_iter().map(|p| p.key).collect()
}

#[test]
fn the_best_match_goes_first() {
    let v = fern("the_best_match_goes_first");
    let k = keys(&v, "How much is the budget for timber?", None, &[]);
    assert_eq!(k.first().map(String::as_str), Some("Projects/Garden Plan.md"), "{k:?}");
    let k = keys(&v, "shelves", None, &[]);
    assert_eq!(k.first().map(String::as_str), Some("Projects/Kitchen/Shelves.md"), "{k:?}");
}

#[test]
fn this_note_means_the_open_one() {
    let v = fern("this_note_means_the_open_one");
    let k = keys(&v, "summarise this note", Some("Reading/Books to read.md"), &[]);
    assert_eq!(k, vec!["Reading/Books to read.md"]);
    let picks = gather(&v, "summarise this note", Some("Reading/Books to read.md"), &[], 6, 12_000);
    assert!(picks[0].open);
}

#[test]
fn a_follow_up_keeps_the_last_notes() {
    let v = fern("a_follow_up_keeps_the_last_notes");
    let before = vec!["Projects/Garden Plan.md".to_string()];
    let k = keys(&v, "and what else?", None, &before);
    assert_eq!(k, before);
    // with nothing before and nothing matching, nothing goes
    assert!(keys(&v, "and what else?", None, &[]).is_empty());
    assert!(words("and what else?").is_empty());
}

#[test]
fn long_notes_send_the_paragraphs_that_match() {
    let mut text = String::from("# Big\n\nThe opening.\n\n");
    for i in 0..200 {
        text.push_str(&format!("## Part {i}\n\nFiller paragraph number {i} with nothing much in it.\n\n"));
    }
    text.push_str("## Plums\n\nThe plum tree goes by the gate.\n");
    let n = obsidian_tui::vault::Note::scratch(&text);
    let out = obsidian_tui::ask::passages(&n, &words("where does the plum tree go"), 800);
    assert!(out.contains("The opening."), "{out}");
    assert!(out.contains("## Plums\n\nThe plum tree goes by the gate."), "{out}");
    assert!(out.contains('…'), "the gap is marked");
    assert!(out.len() <= 801);
}
