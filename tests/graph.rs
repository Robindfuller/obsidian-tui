//! The graph: local and whole-vault pictures, clicking dots, depth, the
//! keyboard and mouse, and that the layout is the same every time and quick.

mod common;
use common::*;

use obsidian_tui::graph::Graph;
use obsidian_tui::harness::H;
use obsidian_tui::vault::Vault;
use ratatui::crossterm::event::{KeyCode, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};

fn ev(kind: MouseEventKind, x: u16, y: u16) -> MouseEvent {
    MouseEvent { kind, column: x, row: y, modifiers: KeyModifiers::NONE }
}

/// Open the vault graph from the sidebar.
fn vault_graph(t: &mut H) -> String {
    let screen = t.render();
    let y = screen.lines().position(|l| l.chars().take(26).collect::<String>().contains(" Graph ")).expect("Graph in the sidebar");
    t.click(3, y as u16)
}

#[test]
fn local_graph_of_a_note() {
    let mut t = h("g-local", 150, 34);
    t.click_text("Welcome");
    let s = t.key(KeyCode::Char('g'));
    golden("graph-local", &s);
    assert_eq!(t.app.tab, 3);
    assert!(s.contains("◉") && s.contains("Welcome"), "the note in the middle stands out");
    for n in ["Garden Plan", "Shelves", "Ideas", "Missing Note"] {
        assert!(s.contains(n), "{n} is linked either way");
    }
    assert!(s.contains("○"), "a missing note is a hollow dot");
    assert!(s.contains("depth 1"));
    // braille lines join them
    assert!(s.chars().any(|c| ('\u{2801}'..='\u{28FF}').contains(&c)));
}

#[test]
fn vault_graph_fills_the_main_area() {
    let mut t = h("g-vault", 150, 30);
    let s = vault_graph(&mut t);
    golden("graph-vault", &s);
    assert_eq!(t.app.view, "graph");
    let g = t.app.geo();
    assert!(g.list.is_none() && g.note.is_none() && g.graph.is_some());
    assert!(s.contains("8 notes · wheel zooms"));
    for n in ["Welcome", "Garden Plan", "Books to read", "2026-09-28"] {
        assert!(s.contains(n), "{n} is in the vault graph");
    }
}

#[test]
fn clicking_a_dot_opens_the_note() {
    let mut t = h("g-click", 150, 34);
    t.click_text("Welcome");
    t.key(KeyCode::Char('g'));
    t.click_in_note("Garden Plan");
    assert_eq!(note(&t), "Projects/Garden Plan.md");
    assert_eq!(t.app.tab, 0, "opens the note to read");
    // a missing note just says so
    t.click_text("Welcome");
    t.key(KeyCode::Char('g'));
    let s = t.click_in_note("Missing Note");
    assert!(s.contains("There's no note called"), "{s}");
    // from the vault graph: back to the list with the note open
    vault_graph(&mut t);
    t.click_text("Books to read");
    assert_eq!(t.app.view, "all");
    assert_eq!(note(&t), "Reading/Books to read.md");
}

#[test]
fn depth_keys_grow_and_shrink_the_local_graph() {
    let mut t = h("g-depth", 150, 34);
    t.click_text("Books to read");
    t.key(KeyCode::Char('g'));
    let n = |t: &mut H| t.app.local_graph().unwrap().nodes.len();
    assert_eq!(n(&mut t), 1, "an unlinked note on its own");
    assert!(t.render().contains("No links in or out yet."));
    // another note keeps the Graph tab
    t.click_text("2026-09-28");
    assert_eq!(t.app.tab, 3);
    let d1 = n(&mut t);
    t.key(KeyCode::Char('+'));
    assert_eq!(t.app.graph_depth, 2);
    let d2 = n(&mut t);
    t.key(KeyCode::Char('+'));
    t.key(KeyCode::Char('+'));
    assert_eq!(t.app.graph_depth, 3, "three hops at most");
    let d3 = n(&mut t);
    assert!(d1 < d2 && d2 <= d3, "{d1} {d2} {d3}");
    t.key(KeyCode::Char('-'));
    t.key(KeyCode::Char('-'));
    t.key(KeyCode::Char('-'));
    assert_eq!(t.app.graph_depth, 1);
    assert_eq!(n(&mut t), d1);
    // the clickable hint does the same
    t.click_in_note("+ more");
    assert_eq!(t.app.graph_depth, 2);
}

#[test]
fn layout_is_the_same_every_time() {
    let (v, _) = vault("g-det");
    let a = Graph::whole(&Vault::open(&v));
    let b = Graph::whole(&Vault::open(&v));
    assert_eq!(a, b);
    let a = Graph::local(&Vault::open(&v), "Welcome.md", 2);
    let b = Graph::local(&Vault::open(&v), "Welcome.md", 2);
    assert_eq!(a, b);
    assert_eq!(a.nodes[a.centre.unwrap()].id, "Welcome.md");
    assert_eq!((a.nodes[0].x, a.nodes[0].y), (0.0, 0.0), "the note sits in the middle");
    // and the picture doesn't change between runs
    let mut t1 = h("g-det-1", 120, 30);
    let mut t2 = h("g-det-2", 120, 30);
    assert_eq!(vault_graph(&mut t1), vault_graph(&mut t2));
}

#[test]
fn hover_highlights_a_dot_and_its_lines() {
    use ratatui::style::Modifier;
    let mut t = h("g-hover", 150, 34);
    t.click_text("Welcome");
    t.key(KeyCode::Char('g'));
    let (x, y) = t.find_from("Garden Plan", 70).unwrap();
    let plain = t.buf.cell((x, y)).unwrap().modifier;
    assert!(!plain.contains(Modifier::BOLD));
    t.app.on_mouse(ev(MouseEventKind::Moved, x, y));
    t.render();
    t.render();
    assert!(t.buf.cell((x, y)).unwrap().modifier.contains(Modifier::BOLD), "the hovered label is picked out");
    // notes not linked to it fade
    let (mx, my) = t.find_from("Missing Note", 70).unwrap();
    assert!(t.buf.cell((mx, my)).unwrap().modifier.contains(Modifier::DIM));
}

#[test]
fn arrows_move_between_dots_enter_opens_esc_goes_back() {
    let mut t = h("g-keys", 150, 34);
    t.click_text("Welcome");
    t.key(KeyCode::Char('g'));
    t.key(KeyCode::Down);
    let first = t.app.graph_sel.clone().expect("a dot is picked");
    assert_ne!(first, "Welcome.md");
    assert_eq!(sel(&t), "Welcome.md", "the list stays put while the graph has the arrows");
    t.key(KeyCode::Right);
    t.key(KeyCode::Left);
    t.key(KeyCode::Up);
    assert!(t.app.graph_sel.is_some());
    let picked = t.app.graph_sel.clone().unwrap();
    t.key(KeyCode::Enter);
    if !picked.starts_with('?') {
        assert_eq!(note(&t), picked);
        assert_eq!(t.app.tab, 0);
    }
    // esc leaves the graph; then up and down move the list again
    t.click_text("Welcome");
    t.key(KeyCode::Char('g'));
    t.key(KeyCode::Esc);
    assert_eq!(t.app.tab, 0);
    t.key(KeyCode::Up);
    assert_eq!(sel(&t), "Projects/Kitchen/Shelves.md");
    // the vault graph: esc goes back to what the sidebar showed
    t.click_text("Recent");
    vault_graph(&mut t);
    t.key(KeyCode::Esc);
    assert_eq!(t.app.view, "recent");
}

#[test]
fn wheel_zooms_drag_pans_zero_resets() {
    let mut t = h("g-mouse", 150, 30);
    vault_graph(&mut t);
    let a = t.app.graph_area;
    let (cx, cy) = (a.x + 5, a.y + a.height - 3);
    t.wheel(cx, cy, false);
    let z = t.app.gview_vault.zoom;
    assert!(z > 1.0, "wheel up zooms in");
    t.wheel(cx, cy, true);
    t.wheel(cx, cy, true);
    assert!(t.app.gview_vault.zoom < z);
    // drag on empty space pans
    let before = t.app.gview_vault;
    t.render();
    t.app.on_mouse(ev(MouseEventKind::Down(MouseButton::Left), cx, cy));
    t.app.on_mouse(ev(MouseEventKind::Drag(MouseButton::Left), cx + 10, cy - 3));
    t.app.on_mouse(ev(MouseEventKind::Up(MouseButton::Left), cx + 10, cy - 3));
    let after = t.app.gview_vault;
    assert!(after.pan_x < before.pan_x && after.pan_y > before.pan_y, "{before:?} {after:?}");
    assert_eq!(t.app.view, "graph", "a drag isn't a click");
    t.key(KeyCode::Char('0'));
    assert_eq!(t.app.gview_vault, obsidian_tui::graph::View::default());
    // + and - zoom the vault graph
    t.key(KeyCode::Char('+'));
    assert!(t.app.gview_vault.zoom > 1.0);
}

#[test]
fn narrow_window_shows_the_graph_full_width() {
    use obsidian_tui::app::Stage;
    let mut t = h("g-narrow", 70, 24);
    t.key(KeyCode::Esc);
    assert_eq!(t.app.shown(), Stage::Rail);
    t.key(KeyCode::Down);
    t.key(KeyCode::Down);
    assert_eq!(t.app.view, "graph");
    let s = t.key(KeyCode::Enter);
    assert!(s.starts_with("╭─ ‹ Graph"));
    assert_eq!(t.app.geo().graph.unwrap().width, 70);
    // and a note's graph, full width too
    t.key(KeyCode::Esc);
    t.key(KeyCode::Enter);
    t.key(KeyCode::Enter);
    let s = t.key(KeyCode::Char('g'));
    assert_eq!(t.app.shown(), Stage::Note);
    assert!(s.contains("depth 1"));
}

#[test]
fn a_big_vault_lays_out_quickly() {
    setup();
    let dir = std::env::temp_dir().join(format!("obsidian-tui-test-{}-g-big", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    // 3,000 notes in 84 folders, four links each, from a fixed sequence
    let mut seed: u64 = 7;
    let mut next = || {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        (seed >> 33) as usize
    };
    for i in 0..3000 {
        let d = dir.join(format!("area{}", i % 12)).join(format!("sub{}", i % 7));
        std::fs::create_dir_all(&d).unwrap();
        let links: Vec<String> = (0..4).map(|_| format!("[[note {}]]", next() % 3000)).collect();
        std::fs::write(d.join(format!("note {i}.md")), format!("# Note {i}\n\n{}\n", links.join(" "))).unwrap();
    }
    let st = dir.with_extension("json");
    let start = std::time::Instant::now();
    let mut t = H::new(&dir, &st, 160, 45);
    let s = vault_graph(&mut t);
    t.wheel(100, 20, false);
    t.render();
    t.click_text("area0");
    t.key(KeyCode::Enter);
    t.key(KeyCode::Char('g'));
    t.key(KeyCode::Char('+'));
    t.key(KeyCode::Char('+'));
    let took = start.elapsed();
    assert!(s.contains("3000 notes"));
    assert!(t.app.local_graph().unwrap().nodes.len() <= obsidian_tui::graph::LOCAL_MAX);
    // debug builds are about ten times slower than release
    let limit = if cfg!(debug_assertions) { 30 } else { 3 };
    assert!(took.as_secs() < limit, "took {took:?}");
    let _ = std::fs::remove_dir_all(&dir);
}
