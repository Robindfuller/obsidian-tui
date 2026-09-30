//! The built-in editor, always on a temp copy of the made-up vault: typing,
//! selections, the clipboard, undo, find, the mouse, and careful saving.

mod common;
use common::*;

use obsidian_tui::harness::H;
use ratatui::crossterm::event::{KeyCode, KeyModifiers};

const CTRL: KeyModifiers = KeyModifiers::CONTROL;
const SHIFT: KeyModifiers = KeyModifiers::SHIFT;

fn editing(name: &str, note: &str) -> (H, std::path::PathBuf) {
    let (v, st) = vault(name);
    let mut t = H::new(&v, &st, 150, 40);
    t.click_text(note);
    t.key(KeyCode::Char('e'));
    assert!(t.app.editor.is_some(), "e opens the editor");
    (t, v)
}

fn text(t: &H) -> String {
    t.app.editor.as_ref().unwrap().text()
}

fn ctrl(t: &mut H, c: char) -> String {
    t.press(KeyCode::Char(c), CTRL)
}

/// The prompt on screen: pick one of its options by its key.
fn answer(t: &mut H, key: &str) {
    let i = t.app.modals.last().expect("a prompt").opts.iter().position(|o| o.as_ref().is_some_and(|(k, _)| k == key)).unwrap();
    t.app.modals.last_mut().unwrap().hi = Some(i);
    t.key(KeyCode::Enter);
}

#[test]
fn type_save_and_see_it_rendered() {
    let (mut t, v) = editing("ed-save", "Books to read");
    let s = t.render();
    assert!(s.contains("Editing: Books to read"));
    assert!(s.contains("^S save") && s.contains("esc done"));
    t.press(KeyCode::End, CTRL);
    t.typ("3. Hedge");
    let s = t.render();
    assert!(s.contains("● not saved"));
    let s = ctrl(&mut t, 's');
    assert!(s.contains("Saved."));
    assert!(!t.app.editor.as_ref().unwrap().modified);
    let on_disk = std::fs::read_to_string(v.join("Reading/Books to read.md")).unwrap();
    assert!(on_disk.ends_with("#reading\n3. Hedge"), "{on_disk:?}");
    // no temp file left behind
    let left: Vec<_> = std::fs::read_dir(v.join("Reading")).unwrap().flatten().map(|e| e.file_name()).collect();
    assert_eq!(left.len(), 1);
    // done: the note shows the new text
    let s = t.key(KeyCode::Esc);
    assert!(t.app.editor.is_none());
    assert!(s.contains("3. Hedge"));
}

#[test]
fn undo_and_redo() {
    let (mut t, _) = editing("ed-undo", "Books to read");
    let before = text(&t);
    t.typ("one two");
    t.key(KeyCode::Backspace);
    assert!(text(&t).starts_with("one tw# Books"));
    ctrl(&mut t, 'z');
    assert!(text(&t).starts_with("one two# Books"));
    ctrl(&mut t, 'z');
    ctrl(&mut t, 'z');
    assert_eq!(text(&t), before);
    assert!(!t.app.editor.as_ref().unwrap().modified, "undone back to the file is not modified");
    ctrl(&mut t, 'y');
    t.press(KeyCode::Char('z'), CTRL | SHIFT);
    assert!(text(&t).starts_with("one two# Books"));
}

#[test]
fn select_copy_cut_paste() {
    let (mut t, _) = editing("ed-clip", "Books to read");
    // "# Books to read": select "# Books"
    for _ in 0..7 {
        t.press(KeyCode::Right, SHIFT);
    }
    assert_eq!(t.app.editor.as_ref().unwrap().selected().as_deref(), Some("# Books"));
    ctrl(&mut t, 'c');
    t.key(KeyCode::End);
    ctrl(&mut t, 'v');
    assert!(text(&t).starts_with("# Books to read# Books\n"));
    // ctrl+x with no selection cuts the line
    ctrl(&mut t, 'x');
    assert!(text(&t).starts_with("\n1. The"));
    ctrl(&mut t, 'v');
    assert!(text(&t).starts_with("# Books to read# Books\n\n1. The"));
    // a word at a time, and everything
    t.press(KeyCode::Home, CTRL);
    t.press(KeyCode::Right, CTRL | SHIFT);
    t.press(KeyCode::Right, CTRL | SHIFT);
    assert_eq!(t.app.editor.as_ref().unwrap().selected().as_deref(), Some("# Books to"));
    ctrl(&mut t, 'a');
    t.typ("x");
    assert_eq!(text(&t), "x");
    // pasting from the terminal (bracketed paste) goes in as typed
    t.app.on_paste("a\r\nb");
    assert_eq!(text(&t), "xa\nb");
}

#[test]
fn find_selects_the_next_match() {
    let (mut t, _) = editing("ed-find", "Welcome");
    ctrl(&mut t, 'f');
    let s = t.typ("compost");
    assert!(s.contains("Find") && s.contains("compost"));
    let ed = t.app.editor.as_ref().unwrap();
    assert_eq!(ed.selected().as_deref(), Some("compost"));
    let first = ed.row;
    t.typ("zzz");
    assert!(t.render().contains("no match"));
    for _ in 0..3 {
        t.key(KeyCode::Backspace);
    }
    t.key(KeyCode::Enter);
    assert_eq!(t.app.editor.as_ref().unwrap().row, first, "the only match: wraps round to itself");
    t.key(KeyCode::Esc);
    assert!(t.app.editor.as_ref().unwrap().find.is_none());
    assert!(t.app.editor.is_some(), "esc closed find, not the editor");
}

#[test]
fn tab_matches_the_file() {
    let (v, st) = vault("ed-tab");
    std::fs::write(v.join("Spaces.md"), "- a\n  - b\n").unwrap();
    std::fs::write(v.join("Tabs.md"), "- a\n\t- b\n").unwrap();
    let mut t = H::new(&v, &st, 150, 40);
    t.click_text("Spaces");
    t.key(KeyCode::Char('e'));
    t.key(KeyCode::Tab);
    assert!(text(&t).starts_with("  - a"));
    t.key(KeyCode::BackTab);
    assert!(text(&t).starts_with("- a"));
    ctrl(&mut t, 's');
    t.key(KeyCode::Esc);
    t.click_text("Tabs");
    t.key(KeyCode::Char('e'));
    t.key(KeyCode::Tab);
    assert!(text(&t).starts_with("\t- a"));
}

#[test]
fn crlf_bom_and_no_final_newline_are_kept() {
    let (v, st) = vault("ed-crlf");
    let orig = b"\xEF\xBB\xBF# Windows note\r\n\r\nFirst line\r\nSecond line";
    std::fs::write(v.join("Windows.md"), orig).unwrap();
    let mut t = H::new(&v, &st, 150, 40);
    t.click_text("Windows");
    t.key(KeyCode::Char('e'));
    assert!(t.render().contains("CRLF"));
    ctrl(&mut t, 's');
    assert_eq!(std::fs::read(v.join("Windows.md")).unwrap(), orig, "saved unchanged: byte for byte");
    t.press(KeyCode::End, CTRL);
    t.key(KeyCode::Enter);
    t.typ("Third line");
    ctrl(&mut t, 's');
    assert_eq!(
        std::fs::read(v.join("Windows.md")).unwrap(),
        b"\xEF\xBB\xBF# Windows note\r\n\r\nFirst line\r\nSecond line\r\nThird line"
    );
    // and a Unix file with a final newline keeps it
    let (mut u, v2) = editing("ed-lf", "Books to read");
    u.key(KeyCode::Down);
    u.typ("x");
    ctrl(&mut u, 's');
    let b = std::fs::read(v2.join("Reading/Books to read.md")).unwrap();
    assert!(b.ends_with(b"#reading\n") && !b.contains(&b'\r'));
}

#[test]
fn changed_on_disk_asks_before_overwriting() {
    let (mut t, v) = editing("ed-disk", "Books to read");
    let p = v.join("Reading/Books to read.md");
    std::fs::write(&p, "someone else's version\n").unwrap();
    t.typ("mine ");
    let s = ctrl(&mut t, 's');
    assert!(s.contains("Changed on disk"));
    answer(&mut t, "keep");
    assert_eq!(std::fs::read_to_string(&p).unwrap(), "someone else's version\n", "not overwritten");
    assert!(t.app.editor.as_ref().unwrap().modified);
    ctrl(&mut t, 's');
    answer(&mut t, "overwrite");
    assert!(std::fs::read_to_string(&p).unwrap().starts_with("mine # Books"));
    // and the other way: take theirs
    std::fs::write(&p, "theirs again\n").unwrap();
    t.typ("more");
    ctrl(&mut t, 's');
    answer(&mut t, "reload");
    assert_eq!(text(&t), "theirs again\n");
    assert!(!t.app.editor.as_ref().unwrap().modified);
    // a file deleted meanwhile counts as changed too
    std::fs::remove_file(&p).unwrap();
    t.typ("x");
    let s = ctrl(&mut t, 's');
    assert!(s.contains("Changed on disk"));
}

#[test]
fn leaving_with_unsaved_changes_asks() {
    let (mut t, v) = editing("ed-leave", "Books to read");
    let p = v.join("Reading/Books to read.md");
    let orig = std::fs::read_to_string(&p).unwrap();
    t.typ("draft ");
    let s = t.key(KeyCode::Esc);
    assert!(s.contains("Save your changes?"));
    answer(&mut t, "keep");
    assert!(t.app.editor.is_some());
    // esc on the prompt is keep editing too
    t.key(KeyCode::Esc);
    t.key(KeyCode::Esc);
    assert!(t.app.editor.is_some() && t.app.modals.is_empty());
    t.key(KeyCode::Esc);
    answer(&mut t, "discard");
    assert!(t.app.editor.is_none());
    assert_eq!(std::fs::read_to_string(&p).unwrap(), orig);
    t.key(KeyCode::Char('e'));
    t.typ("kept ");
    t.key(KeyCode::Esc);
    answer(&mut t, "save");
    assert!(t.app.editor.is_none());
    assert!(std::fs::read_to_string(&p).unwrap().starts_with("kept # Books"));
    // nothing changed: esc just leaves
    t.key(KeyCode::Char('e'));
    t.key(KeyCode::Esc);
    assert!(t.app.editor.is_none() && t.app.modals.is_empty());
}

#[test]
fn arrows_edit_while_editing_and_move_the_list_after() {
    let (mut t, _) = editing("ed-arrows", "Garden Plan");
    t.key(KeyCode::Down);
    t.key(KeyCode::Down);
    assert_eq!(sel(&t), "Projects/Garden Plan.md", "the list stays put while editing");
    assert_eq!(t.app.editor.as_ref().unwrap().row, 2);
    // q types, ctrl+c copies: neither quits
    t.typ("q");
    ctrl(&mut t, 'c');
    assert!(!t.app.quit);
    t.key(KeyCode::Backspace);
    t.key(KeyCode::Esc);
    assert!(t.app.editor.is_none());
    t.key(KeyCode::Down);
    assert_eq!(sel(&t), "Ideas.md");
}

#[test]
fn mouse_places_the_cursor_selects_and_scrolls() {
    let (mut t, _) = editing("ed-mouse", "Garden Plan");
    use ratatui::crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
    let (x, y) = t.find("Four raised").unwrap();
    let ev = |kind, x, y| MouseEvent { kind, column: x, row: y, modifiers: KeyModifiers::NONE };
    t.app.on_mouse(ev(MouseEventKind::Down(MouseButton::Left), x + 5, y));
    t.app.on_mouse(ev(MouseEventKind::Up(MouseButton::Left), x + 5, y));
    let ed = t.app.editor.as_ref().unwrap();
    assert_eq!(ed.lines[ed.row][..ed.col].iter().collect::<String>(), "Four ");
    assert!(ed.sel().is_none());
    t.app.on_mouse(ev(MouseEventKind::Down(MouseButton::Left), x, y));
    t.app.on_mouse(ev(MouseEventKind::Drag(MouseButton::Left), x + 11, y));
    t.app.on_mouse(ev(MouseEventKind::Up(MouseButton::Left), x + 11, y));
    assert_eq!(t.app.editor.as_ref().unwrap().selected().as_deref(), Some("Four raised"));
    // clicks outside the editor don't wander off to another note
    t.click_text("Welcome");
    assert!(t.app.editor.is_some());
    assert_eq!(note(&t), "Projects/Garden Plan.md");
    t.app.on_mouse(ev(MouseEventKind::ScrollDown, x, y));
    assert!(t.app.editor.as_ref().unwrap().scroll > 0);
}

#[test]
fn long_lines_wrap_and_down_follows_them() {
    let (v, st) = vault("ed-wrap");
    std::fs::write(v.join("Long.md"), format!("{}\nend", "word ".repeat(60))).unwrap();
    let mut t = H::new(&v, &st, 120, 30);
    t.click_text("Long");
    t.key(KeyCode::Char('e'));
    let s = t.render();
    assert!(s.lines().filter(|l| l.contains("word word")).count() >= 3, "one line, shown on several");
    t.key(KeyCode::Down);
    let ed = t.app.editor.as_ref().unwrap();
    assert_eq!(ed.row, 0, "down moved within the wrapped line");
    assert!(ed.col > 0);
}

#[test]
fn not_utf8_is_left_alone() {
    let (v, st) = vault("ed-bin");
    std::fs::write(v.join("Latin.md"), b"caf\xe9\n").unwrap();
    let mut t = H::new(&v, &st, 150, 40);
    t.click_text("Latin");
    let s = t.key(KeyCode::Char('e'));
    assert!(t.app.editor.is_none());
    assert!(s.contains("isn't UTF-8"));
    assert_eq!(std::fs::read(v.join("Latin.md")).unwrap(), b"caf\xe9\n");
}

#[test]
fn narrow_window_edits_full_width() {
    let (v, st) = vault("ed-narrow");
    let mut t = H::new(&v, &st, 70, 24);
    t.key(KeyCode::Enter);
    let s = t.key(KeyCode::Char('e'));
    assert!(s.lines().next().unwrap().starts_with("╭─ Editing:"));
    t.key(KeyCode::Esc);
    assert_eq!(t.app.shown(), obsidian_tui::app::Stage::Note);
}
