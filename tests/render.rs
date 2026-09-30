//! Whole screens of the made-up vault, compared with the saved copies in
//! tests/golden (UPDATE_GOLDEN=1 rewrites them).

mod common;
use common::*;

use ratatui::crossterm::event::KeyCode;

#[test]
fn opens_on_the_first_note() {
    let mut t = h("first", 150, 40);
    let s = t.render();
    golden("all-notes", &s);
    assert_eq!(sel(&t), "Daily/2026-09-28.md");
    assert!(s.contains("Fern Vault"));
    // .trash and .obsidian are not part of the vault
    assert!(!s.contains("Old note"));
    assert!(s.contains("8 notes"));
}

#[test]
fn welcome_note() {
    let mut t = h("welcome", 150, 60);
    let s = t.click_text("Welcome");
    golden("welcome", &s);
    assert!(s.contains("Welcome to the Fern Vault"));
    assert!(s.contains("│ Tip of the day"), "callout title with its bar");
    assert!(s.contains("[x] Sweep the shed"));
    assert!(s.contains("│ North │ Carrots │    4 │"), "right-aligned column");
    assert!(s.contains("#projects/garden"));
    assert!(!s.contains("%%"));
}

#[test]
fn welcome_bottom() {
    let mut t = h("welcome-end", 150, 40);
    t.click_text("Welcome");
    let s = t.key(KeyCode::End);
    golden("welcome-end", &s);
    assert!(s.contains("def water(fern):"));
    assert!(s.contains("↳ diagram.png"));
}

#[test]
fn links_and_outline_tabs() {
    let mut t = h("tabs", 150, 36);
    t.click_text("Welcome");
    let s = t.key(KeyCode::Char('2'));
    golden("links-tab", &s);
    assert!(s.contains("LINKED MENTIONS · 2"));
    assert!(s.contains("Missing Note  (no such note)"));
    let s = t.key(KeyCode::Char('3'));
    golden("outline-tab", &s);
    assert!(s.contains("Tasks") && s.contains("Table") && s.contains("Code"));
}

#[test]
fn search_results() {
    let mut t = h("search", 150, 30);
    t.key(KeyCode::Char('/'));
    t.typ("compost");
    let s = t.key(KeyCode::Enter);
    golden("search", &s);
    assert!(s.contains("Search: compost"));
    assert!(s.contains("3 notes · best first"));
    assert!(s.contains("n next match"));
}

#[test]
fn narrow_terminal_swaps_list_and_note() {
    let mut t = h("narrow", 80, 24);
    let s = t.render();
    golden("narrow-list", &s);
    assert!(s.contains("All notes") && !s.contains("edited"));
    let s = t.key(KeyCode::Enter);
    golden("narrow-note", &s);
    assert!(s.contains("esc back"));
    let s = t.key(KeyCode::Esc);
    assert!(s.contains("8 notes"));
}

#[test]
fn folded_sidebar_and_menus() {
    let mut t = h("fold", 120, 30);
    let s = t.key(KeyCode::Char('b'));
    golden("folded", &s);
    assert!(s.contains(" »"));
    let s = t.key(KeyCode::Char('?'));
    golden("keys", &s);
    assert!(s.contains("step through links"));
    t.key(KeyCode::Esc);
    let (x, y) = t.find("Garden Plan").unwrap();
    let s = t.rclick(x, y);
    golden("row-menu", &s);
    assert!(s.contains("Open in Obsidian"));
}
