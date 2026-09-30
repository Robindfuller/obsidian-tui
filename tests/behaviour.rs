//! What keys and clicks do: arrows after any click or scroll, following links,
//! history, tags, search, and never writing to the vault.

mod common;
use common::*;

use obsidian_tui::app::Focus;
use obsidian_tui::vault::Vault;
use ratatui::crossterm::event::{KeyCode, KeyModifiers};

// ------------------------------------------------------------ arrows

#[test]
fn arrows_move_the_list_after_clicking_a_row() {
    let mut t = h("arrow-row", 150, 40);
    t.click_text("Garden Plan");
    assert_eq!(sel(&t), "Projects/Garden Plan.md");
    t.key(KeyCode::Down);
    assert_eq!(sel(&t), "Ideas.md");
    assert_eq!(note(&t), "Ideas.md", "moving opens the note beside the list");
    t.key(KeyCode::Up);
    assert_eq!(sel(&t), "Projects/Garden Plan.md");
}

#[test]
fn arrows_move_the_list_after_clicking_the_note() {
    let mut t = h("arrow-note", 150, 40);
    t.click_text("Welcome");
    // clicked some plain text in the note body: still the list's keys
    let (x, y) = t.find_from("Welcome to the Fern Vault", 70).unwrap();
    t.click(x, y);
    t.key(KeyCode::Up);
    assert_eq!(sel(&t), "Projects/Kitchen/Shelves.md");
}

#[test]
fn arrows_move_the_list_after_clicking_the_sidebar() {
    let mut t = h("arrow-rail", 150, 40);
    t.click_text("Projects");
    assert_eq!(t.app.view, "folder:Projects");
    assert_eq!(t.app.focus, Focus::None);
    let first = sel(&t);
    t.key(KeyCode::Down);
    assert_ne!(sel(&t), first, "down moved the list, not the sidebar");
    assert_eq!(t.app.view, "folder:Projects");
}

#[test]
fn arrows_move_the_list_after_scrolling() {
    let mut t = h("arrow-wheel", 150, 30);
    t.click_text("Welcome");
    let (x, y) = t.find_from("Welcome to the Fern Vault", 70).unwrap();
    t.wheel(x, y, true);
    assert!(t.app.note_scroll > 0);
    t.wheel(30, 5, true); // over the list
    t.wheel(3, 5, true); // over the sidebar
    t.key(KeyCode::Up);
    assert_eq!(sel(&t), "Projects/Kitchen/Shelves.md");
}

#[test]
fn arrows_move_the_list_after_following_a_link() {
    let mut t = h("arrow-link", 150, 40);
    t.click_text("Welcome");
    t.click_in_note("shelf project");
    assert_eq!(note(&t), "Projects/Kitchen/Shelves.md");
    t.key(KeyCode::Down);
    assert_eq!(sel(&t), "Welcome.md", "the list moves on from its own selection");
}

#[test]
fn arrows_move_the_list_while_filtering() {
    let mut t = h("arrow-filter", 150, 40);
    t.key(KeyCode::Char('/'));
    t.typ("i");
    assert_eq!(t.app.focus, Focus::Filter);
    let first = sel(&t);
    t.key(KeyCode::Down);
    assert_ne!(sel(&t), first);
}

#[test]
fn left_moves_to_the_sidebar_on_purpose_only() {
    let mut t = h("rail-keys", 150, 40);
    t.key(KeyCode::Left);
    assert_eq!(t.app.focus, Focus::Rail);
    t.key(KeyCode::Down);
    assert_eq!(t.app.view, "recent");
    t.key(KeyCode::Right);
    assert_eq!(t.app.focus, Focus::None);
    // and any click hands the arrows back to the list
    t.key(KeyCode::Left);
    t.click_text("Books to read");
    assert_eq!(t.app.focus, Focus::None);
}

// ------------------------------------------------------------ links

#[test]
fn links_resolve_like_obsidian() {
    let (v, _) = vault("resolve");
    let vault = Vault::open(&v);
    let key = |t: &str, from: &str| vault.resolve(t, Some(from)).map(|i| vault.notes[i].key.clone());
    assert_eq!(key("Garden Plan", "Welcome.md").as_deref(), Some("Projects/Garden Plan.md"));
    assert_eq!(key("Projects/Kitchen/Shelves", "Welcome.md").as_deref(), Some("Projects/Kitchen/Shelves.md"));
    assert_eq!(key("Kitchen/Shelves", "Welcome.md").as_deref(), Some("Projects/Kitchen/Shelves.md"), "a partial path");
    assert_eq!(key("Shelves.md", "Daily/2026-09-29.md").as_deref(), Some("Projects/Kitchen/Shelves.md"));
    assert_eq!(key("Projects\\Kitchen\\Shelves", "Welcome.md").as_deref(), Some("Projects/Kitchen/Shelves.md"), "Windows slashes");
    assert_eq!(key("garden plan", "Welcome.md").as_deref(), Some("Projects/Garden Plan.md"), "any case");
    assert_eq!(key("Brainstorm", "Welcome.md").as_deref(), Some("Ideas.md"), "an alias");
    assert_eq!(key("Home", "Ideas.md").as_deref(), Some("Welcome.md"), "an alias in a list");
    // two notes called Ideas: the one beside the linking note wins
    assert_eq!(key("Ideas", "Welcome.md").as_deref(), Some("Ideas.md"));
    assert_eq!(key("Ideas", "Projects/Garden Plan.md").as_deref(), Some("Projects/Ideas.md"));
    assert_eq!(key("", "Welcome.md").as_deref(), Some("Welcome.md"), "[[#heading]] is this note");
    assert_eq!(key("Missing Note", "Welcome.md"), None);
    assert_eq!(vault.resolve_file("diagram.png", Some("Welcome.md")).as_deref(), Some("attachments/diagram.png"));
}

#[test]
fn backlinks_tags_and_code_are_indexed() {
    let (v, _) = vault("index");
    let vault = Vault::open(&v);
    let back: Vec<&str> = vault.backlinks["Welcome.md"].iter().map(|(k, _)| k.as_str()).collect();
    assert_eq!(back, vec!["Ideas.md", "Projects/Garden Plan.md"]);
    let shelves = vault.note("Projects/Kitchen/Shelves.md").unwrap();
    assert!(!shelves.links.iter().any(|l| l.target == "Not a link"), "links in code don't count");
    assert!(!vault.tags.contains_key("not-a-tag"), "tags in code don't count");
    assert!(vault.tags.contains_key("projects/garden"));
    assert_eq!(vault.tagged("projects").len(), 2, "a parent tag takes in its children");
    assert_eq!(vault.notes.len(), 8);
    assert!(vault.files.contains(&"attachments/diagram.png".to_string()));
}

#[test]
fn clicking_links_opens_notes_headings_and_history() {
    let mut t = h("follow", 150, 40);
    t.click_text("Welcome");
    t.click_in_note("Garden Plan#Beds");
    assert_eq!(note(&t), "Projects/Garden Plan.md");
    let s = t.render();
    assert!(s.contains("Four raised beds"), "scrolled to the Beds heading");
    t.click_in_note("Welcome");
    assert_eq!(note(&t), "Welcome.md");
    t.key(KeyCode::Char('['));
    assert_eq!(note(&t), "Projects/Garden Plan.md");
    t.key(KeyCode::Char('['));
    assert_eq!(note(&t), "Welcome.md");
    t.press(KeyCode::Right, KeyModifiers::ALT);
    assert_eq!(note(&t), "Projects/Garden Plan.md");
    t.key(KeyCode::Char(']'));
    assert_eq!(note(&t), "Welcome.md");
}

#[test]
fn missing_links_urls_embeds_and_tags() {
    let mut t = h("kinds", 150, 60);
    t.click_text("Welcome");
    let s = t.click_in_note("Missing Note");
    assert!(s.contains("no note called “Missing Note”"));
    assert_eq!(note(&t), "Welcome.md");
    t.app.toasts.clear();
    t.click_in_note("Obsidian.");
    assert_eq!(t.app.opened.last().map(|s| s.as_str()), Some("https://obsidian.md"));
    t.click_in_note("↳ diagram.png");
    assert!(t.app.opened.last().unwrap().ends_with("diagram.png"));
    t.click_in_note("#inbox");
    assert_eq!(t.app.view, "tag:inbox");
    assert_eq!(t.app.rows.len(), 1);
    t.click_text("#journal");
    assert_eq!(t.app.view, "tag:journal");
    assert_eq!(t.app.rows.len(), 2);
}

#[test]
fn tab_steps_through_links_and_enter_follows() {
    let mut t = h("tabkey", 150, 60);
    t.click_text("Welcome");
    t.key(KeyCode::Tab);
    assert_eq!(t.app.link_sel, Some(0));
    // the first links are the frontmatter tags, then Garden Plan
    let links = t.app.body_links();
    let i = links.iter().position(|(_, a)| matches!(a, obsidian_tui::rich::Act::Open { key, heading: None } if key == "Projects/Garden Plan.md")).unwrap();
    for _ in 0..i {
        t.key(KeyCode::Tab);
    }
    t.key(KeyCode::Enter);
    assert_eq!(note(&t), "Projects/Garden Plan.md");
    t.key(KeyCode::BackTab);
    assert!(t.app.link_sel.is_some());
    t.key(KeyCode::Esc);
    assert_eq!(t.app.link_sel, None);
}

#[test]
fn outline_jumps_to_a_heading() {
    let mut t = h("outline", 150, 30);
    t.click_text("Welcome");
    t.key(KeyCode::Char('3'));
    t.click_in_note("Code");
    assert_eq!(t.app.tab, 0);
    let s = t.render();
    assert!(s.contains("def water(fern):"));
}

// ------------------------------------------------------------ search

#[test]
fn filter_narrows_titles_as_you_type() {
    let mut t = h("filter", 150, 40);
    t.key(KeyCode::Char('/'));
    t.typ("shel");
    assert_eq!(t.app.rows.len(), 1);
    assert_eq!(sel(&t), "Projects/Kitchen/Shelves.md");
    t.key(KeyCode::Esc);
    assert_eq!(t.app.rows.len(), 8);
}

#[test]
fn full_text_search_marks_and_steps_through_hits() {
    let mut t = h("fulltext", 150, 24);
    t.key(KeyCode::Char('/'));
    t.typ("compost");
    t.key(KeyCode::Enter);
    assert_eq!(t.app.view, "search");
    let keys: Vec<String> = t.app.rows.iter().map(|r| r.key.clone()).collect();
    assert_eq!(keys, vec!["Daily/2026-09-29.md", "Projects/Garden Plan.md", "Welcome.md"]);
    assert_eq!(t.app.terms, vec!["compost"]);
    t.key(KeyCode::Down);
    t.key(KeyCode::Down);
    assert_eq!(note(&t), "Welcome.md");
    let w = t.app.body_width();
    let r = t.app.rendered(w).unwrap();
    assert_eq!(r.hits.len(), 1, "the one line that says compost is marked");
    t.key(KeyCode::Char('n'));
    assert_eq!(t.app.note_scroll, r.hits[0].saturating_sub(2));
    // a phrase in quotes, and nothing found
    t.key(KeyCode::Char('/'));
    t.typ("\"raised beds\"");
    t.key(KeyCode::Enter);
    assert_eq!(t.app.rows.len(), 1);
    t.key(KeyCode::Char('/'));
    t.typ("zebra");
    let s = t.key(KeyCode::Enter);
    assert!(s.contains("Nothing in the vault says"));
    t.key(KeyCode::Esc);
    t.key(KeyCode::Esc);
    assert_eq!(t.app.view, "all");
}

#[test]
fn sort_by_date_and_recent() {
    let mut t = h("sort", 150, 40);
    t.key(KeyCode::Char('s'));
    assert_eq!(t.app.rows[0].key, "Welcome.md", "newest first");
    t.key(KeyCode::Char('s'));
    assert_eq!(t.app.rows[0].key, "Daily/2026-09-28.md");
    t.click_text("Recent");
    assert_eq!(t.app.rows[0].key, "Welcome.md");
}

// ------------------------------------------------------------ outside

#[test]
fn edit_hands_the_path_to_the_editor_and_nothing_is_written() {
    let (v, st) = vault("readonly");
    let before = snapshot(&v);
    let mut t = obsidian_tui::harness::H::new(&v, &st, 150, 40);
    t.click_text("Welcome");
    t.key(KeyCode::Char('e'));
    assert_eq!(t.app.pending_edit.as_deref(), Some(v.join("Welcome.md").as_path()));
    t.key(KeyCode::Char('o'));
    assert_eq!(t.app.opened.last().unwrap(), "obsidian://open?vault=Fern%20Vault&file=Welcome");
    t.key(KeyCode::Char('y'));
    assert!(t.app.pending_out.iter().any(|s| s.starts_with("\x1b]52;c;")));
    // wander about: every tab, search, sidebar, menus
    for k in ['2', '3', '1', '/', 'x', '\n', 'b', 'b', '?'] {
        let code = if k == '\n' { KeyCode::Enter } else { KeyCode::Char(k) };
        t.key(code);
    }
    t.key(KeyCode::Esc);
    t.app.save_state();
    assert_eq!(snapshot(&v), before, "the vault is exactly as it was");
    assert!(st.exists(), "state lives outside the vault");
}

#[test]
fn rescan_picks_up_new_and_deleted_notes() {
    let (v, st) = vault("rescan");
    let mut t = obsidian_tui::harness::H::new(&v, &st, 150, 40);
    std::fs::write(v.join("Daily/2026-09-30.md"), "# Tuesday\n\nNew day. [[Welcome]]\n").unwrap();
    t.app.rescan_now();
    assert_eq!(t.app.rows.len(), 9);
    assert_eq!(t.app.vault.backlinks["Welcome.md"].len(), 3);
    std::fs::remove_file(v.join("Ideas.md")).unwrap();
    t.app.rescan_now();
    assert_eq!(t.app.rows.len(), 8);
}

#[test]
fn state_is_remembered_per_vault() {
    let (v, st) = vault("state");
    {
        let mut t = obsidian_tui::harness::H::new(&v, &st, 150, 40);
        t.click_text("Projects");
        t.click_text("Shelves");
        t.key(KeyCode::Char('b'));
        t.app.save_state();
    }
    let t = obsidian_tui::harness::H::new(&v, &st, 150, 40);
    assert_eq!(t.app.view, "folder:Projects");
    assert_eq!(note(&t), "Projects/Kitchen/Shelves.md");
    assert!(t.app.rail_collapsed);
}

#[test]
fn folders_fold_and_open() {
    let mut t = h("folders", 150, 40);
    let rail = |t: &mut obsidian_tui::harness::H| -> String {
        t.render().lines().map(|l| l.chars().take(26).collect::<String>() + "\n").collect()
    };
    assert!(!rail(&mut t).contains("Kitchen"));
    let (x, y) = t.find("▸ Projects").unwrap();
    t.click(x, y);
    assert!(rail(&mut t).contains("Kitchen"), "the arrow opens the folder");
    assert_eq!(t.app.view, "all", "without changing the list");
    let (x, y) = t.find("▾ Projects").unwrap();
    t.click(x, y);
    assert!(!rail(&mut t).contains("Kitchen"));
    t.click_text("Projects");
    t.click_text("Kitchen");
    assert_eq!(t.app.view, "folder:Projects/Kitchen");
    assert_eq!(t.app.rows.len(), 1);
}

#[test]
fn dragging_the_edge_resizes_the_list() {
    use ratatui::crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
    let mut t = h("drag", 150, 40);
    t.render();
    let ev = |kind, x| MouseEvent { kind, column: x, row: 10, modifiers: KeyModifiers::NONE };
    let edge = t.app.divider_x().unwrap();
    t.app.on_mouse(ev(MouseEventKind::Down(MouseButton::Left), edge));
    assert!(t.app.dragging);
    t.app.on_mouse(ev(MouseEventKind::Drag(MouseButton::Left), edge + 20));
    t.render();
    assert_eq!(t.app.divider_x(), Some(edge + 20), "the list follows the mouse");
    // never so wide the note can't be read
    t.app.on_mouse(ev(MouseEventKind::Drag(MouseButton::Left), 149));
    let g = t.app.geo();
    assert!(g.note.unwrap().width >= 40);
    t.app.on_mouse(ev(MouseEventKind::Up(MouseButton::Left), 149));
    assert!(!t.app.dragging);
    assert_eq!(t.app.sel, Some("Daily/2026-09-28.md".into()), "a drag isn't a click on a row");
    // up and down still move the list afterwards
    t.key(KeyCode::Down);
    assert_eq!(sel(&t), "Daily/2026-09-29.md");
    // a narrow window has nothing to drag: the list, then enter for the note
    let mut n = h("drag-narrow", 80, 24);
    n.render();
    assert_eq!(n.app.divider_x(), None);
    n.key(KeyCode::Enter);
    assert!(n.app.geo().list.is_none() && n.app.geo().note.is_some());
}

#[test]
fn narrow_window_steps_folder_then_notes_then_note() {
    use obsidian_tui::app::Stage;
    let mut t = h("stages", 70, 24);
    // opens on the notes; esc steps back to the sidebar, full width
    assert_eq!(t.app.shown(), Stage::List);
    let s = t.key(KeyCode::Esc);
    assert_eq!(t.app.shown(), Stage::Rail);
    assert!(s.contains("FOLDERS") && !s.contains("filter"));
    golden("narrow-rail", &s);
    // up and down move through the sidebar without leaving it
    t.key(KeyCode::Down);
    t.key(KeyCode::Down);
    t.key(KeyCode::Down);
    assert_eq!(t.app.view, "folder:Projects");
    assert_eq!(t.app.shown(), Stage::Rail);
    // → on a closed folder opens it; enter goes in
    t.key(KeyCode::Char('h'));
    t.key(KeyCode::Right);
    assert!(t.app.open_folders.contains("Projects"));
    assert_eq!(t.app.shown(), Stage::Rail);
    let s = t.key(KeyCode::Enter);
    assert_eq!(t.app.shown(), Stage::List);
    assert!(s.contains("‹ Projects") && s.contains("3 notes"));
    // the list: arrows move, enter reads
    t.key(KeyCode::Down);
    assert_eq!(sel(&t), "Projects/Ideas.md");
    t.key(KeyCode::Up);
    let s = t.key(KeyCode::Enter);
    assert_eq!(t.app.shown(), Stage::Note);
    assert!(s.contains("The plan for the spring"));
    // in the note, arrows scroll it rather than changing note
    t.key(KeyCode::Down);
    assert_eq!(t.app.note_scroll, 1);
    assert_eq!(note(&t), "Projects/Garden Plan.md");
    // links still work, and back steps out one at a time
    t.click_text("Welcome.");
    assert_eq!(note(&t), "Welcome.md");
    assert_eq!(t.app.shown(), Stage::Note);
    t.key(KeyCode::Left);
    assert_eq!(t.app.shown(), Stage::List);
    t.key(KeyCode::Esc);
    assert_eq!(t.app.shown(), Stage::Rail);
}

#[test]
fn narrow_window_steps_by_clicking() {
    use obsidian_tui::app::Stage;
    let mut t = h("stage-clicks", 70, 24);
    t.click_text("‹ All notes");
    assert_eq!(t.app.shown(), Stage::Rail);
    t.click_text("#journal");
    assert_eq!(t.app.shown(), Stage::List);
    assert_eq!(t.app.rows.len(), 2);
    t.click_text("2026-09-29");
    assert_eq!(t.app.shown(), Stage::Note);
    t.click_text("‹ notes");
    assert_eq!(t.app.shown(), Stage::List);
    // search from the sidebar shows the list to type into
    t.key(KeyCode::Esc);
    t.key(KeyCode::Char('/'));
    assert_eq!(t.app.shown(), Stage::List);
    t.typ("compost");
    t.key(KeyCode::Enter);
    assert_eq!(t.app.rows.len(), 3);
    // a wide window shows all three at once again
    t.app.size = (150, 40);
    let g = t.app.geo();
    assert!(g.rail.width > 0 && g.list.is_some() && g.note.is_some());
}
