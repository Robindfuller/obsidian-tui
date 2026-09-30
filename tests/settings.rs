//! The Settings panel: opening it, changing things with keys and clicks,
//! typing text settings, and every change kept in the state file.

mod common;
use common::*;

use obsidian_tui::app::App;
use obsidian_tui::ask::{Conf, prompt};
use ratatui::crossterm::event::{KeyCode, KeyModifiers};

fn saved(path: &std::path::Path) -> serde_json::Value {
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

#[test]
fn comma_opens_it_and_esc_closes_it() {
    let mut t = h("set-open", 150, 34);
    let s = t.key(KeyCode::Char(','));
    golden("settings", &s);
    assert!(t.app.settings.is_some());
    assert!(s.contains("Notes per question      ‹ 6 ›"));
    t.key(KeyCode::Esc);
    assert!(t.app.settings.is_none());
}

#[test]
fn arrows_change_numbers_and_it_is_saved() {
    let (v, st) = vault("set-arrows");
    let mut t = obsidian_tui::harness::H::new(&v, &st, 150, 34);
    t.key(KeyCode::Char(','));
    t.key(KeyCode::Down);
    t.key(KeyCode::Down);
    t.key(KeyCode::Right);
    t.key(KeyCode::Right);
    assert_eq!(t.app.ask_conf.notes, 8);
    t.key(KeyCode::Down);
    let s = t.key(KeyCode::Right);
    assert_eq!(t.app.ask_conf.local_chars, 16_000);
    assert!(s.contains("‹ 16,000 characters ›"));
    // think is on or off
    for _ in 0..3 {
        t.key(KeyCode::Down);
    }
    t.key(KeyCode::Enter);
    assert!(t.app.ask_conf.think);
    let j = saved(&st);
    assert_eq!(j["ask"]["notes"], 8);
    assert_eq!(j["ask"]["local_chars"], 16_000);
    assert_eq!(j["ask"]["think"], true);
    // and it's there next time
    let again = App::with_state(&v, st.clone());
    assert_eq!(again.ask_conf.notes, 8);
    assert!(again.ask_conf.think);
}

#[test]
fn the_ends_stop_the_numbers() {
    let mut t = h("set-ends", 150, 34);
    t.key(KeyCode::Char(','));
    t.key(KeyCode::Down);
    t.key(KeyCode::Down);
    for _ in 0..30 {
        t.key(KeyCode::Left);
    }
    assert_eq!(t.app.ask_conf.notes, 1);
    for _ in 0..30 {
        t.key(KeyCode::Right);
    }
    assert_eq!(t.app.ask_conf.notes, 20);
}

#[test]
fn text_settings_are_typed_in() {
    let (v, st) = vault("set-text");
    let mut t = obsidian_tui::harness::H::new(&v, &st, 150, 34);
    t.key(KeyCode::Char(','));
    for _ in 0..8 {
        t.key(KeyCode::Down);
    }
    t.key(KeyCode::Enter);
    t.typ("Answer in bullet points");
    assert!(t.app.ask_conf.extra.is_empty(), "not kept until enter");
    let s = t.key(KeyCode::Enter);
    assert_eq!(t.app.ask_conf.extra, "Answer in bullet points");
    assert!(s.contains("Extra instructions      Answer in bullet points"));
    assert_eq!(saved(&st)["ask"]["extra"], "Answer in bullet points");
    // esc while typing leaves it as it was
    t.key(KeyCode::Enter);
    t.typ(" and rhyme");
    t.key(KeyCode::Esc);
    assert_eq!(t.app.ask_conf.extra, "Answer in bullet points");
    assert!(t.app.settings.is_some(), "esc left the typing, not the panel");
    // and the model is told
    let (system, _) = prompt(&t.app.vault, &[], "hi", &[], &t.app.ask_conf);
    assert!(system.ends_with("Answer in bullet points"), "{system}");
}

#[test]
fn reset_puts_it_back() {
    let mut t = h("set-reset", 150, 34);
    t.key(KeyCode::Char(','));
    t.key(KeyCode::Down);
    t.key(KeyCode::Down);
    t.key(KeyCode::Right);
    assert_ne!(t.app.ask_conf, Conf::default());
    t.key(KeyCode::End);
    t.key(KeyCode::Enter);
    assert_eq!(t.app.ask_conf, Conf::default());
}

#[test]
fn clicks_work_too() {
    let mut t = h("set-click", 150, 34);
    t.key(KeyCode::Char(','));
    let (x, y) = t.find("‹ 6 ›").unwrap();
    t.click(x + 4, y);
    assert_eq!(t.app.ask_conf.notes, 7);
    t.click(x, y);
    t.click(x, y);
    assert_eq!(t.app.ask_conf.notes, 5);
    // the model opens the list, and esc there goes back to the panel
    let s = t.click_text("test model ▾");
    assert!(s.contains("Answer with"));
    t.key(KeyCode::Esc);
    assert!(t.app.modals.is_empty() && t.app.settings.is_some());
    // a click outside closes it
    t.click(2, 2);
    assert!(t.app.settings.is_none());
}

#[test]
fn ctrl_o_opens_it_from_the_ask_pane() {
    let mut t = h("set-ask", 150, 34);
    t.key(KeyCode::Char('a'));
    t.typ("a,b");
    assert!(t.app.settings.is_none(), "a comma in a question is just a comma");
    t.press(KeyCode::Char('o'), KeyModifiers::CONTROL);
    assert!(t.app.settings.is_some());
    t.key(KeyCode::Char('q'));
    assert!(t.app.settings.is_none());
    assert!(t.app.asking, "back in the ask pane");
    assert_eq!(t.app.chat.input.value(), "a,b");
    // and the link in its head
    t.click_text("settings");
    assert!(t.app.settings.is_some());
}

#[test]
fn the_old_model_setting_is_read() {
    let (v, st) = vault("set-old");
    std::fs::write(&st, r#"{"ask_model": "ollama:llama3.2:3b"}"#).unwrap();
    let mut app = App::with_state(&v, st.clone());
    app.headless = true;
    app.save_state();
    let j = saved(&st);
    assert_eq!(j["ask"]["model"], "ollama:llama3.2:3b");
    assert!(j.get("ask_model").is_none());
}

#[test]
fn notes_per_question_limits_what_goes() {
    let (v, _) = vault("set-limit");
    let vault = obsidian_tui::vault::Vault::open(&v);
    let picks = obsidian_tui::ask::gather(&vault, "garden plan ideas welcome", None, &[], 1, 12_000);
    assert_eq!(picks.len(), 1);
}
