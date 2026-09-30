//! The vault is watched: a note written by something else shows up.

mod common;
use common::*;

use std::time::{Duration, Instant};

#[test]
fn a_new_note_shows_up_by_itself() {
    let (v, st) = vault("watch");
    let mut t = obsidian_tui::harness::H::new(&v, &st, 150, 40);
    t.app.headless = false;
    t.app.start();
    t.app.headless = true;
    std::thread::sleep(Duration::from_millis(200));
    std::fs::write(v.join("Arrived.md"), "# Arrived\n").unwrap();
    let end = Instant::now() + Duration::from_secs(5);
    while Instant::now() < end && t.app.vault.note("Arrived.md").is_none() {
        std::thread::sleep(Duration::from_millis(50));
        t.app.tick();
    }
    assert!(t.app.vault.note("Arrived.md").is_some());
    // changes inside .obsidian are ignored
    let generation = t.app.vault.generation;
    std::fs::write(v.join(".obsidian/workspace.json"), "{}").unwrap();
    std::thread::sleep(Duration::from_millis(600));
    t.app.tick();
    assert_eq!(t.app.vault.generation, generation);
}
