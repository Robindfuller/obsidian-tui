//! On Omarchy the colours come from the live theme and follow a switch;
//! anywhere else the built-in palette is used.

use std::path::PathBuf;

use obsidian_tui::theme::Theme;

fn write_theme(dir: &std::path::Path, name: &str, bg: &str) {
    std::fs::create_dir_all(dir.join("theme")).unwrap();
    std::fs::write(dir.join("theme.name"), name).unwrap();
    std::fs::write(
        dir.join("theme/colors.toml"),
        format!("background = \"{bg}\"\nforeground = \"#e0e0e0\"\naccent = \"#88c0d0\"\nred = \"#bf616a\"\ngreen = \"#a3be8c\"\nyellow = \"#ebcb8b\"\nblue = \"#81a1c1\"\n"),
    )
    .unwrap();
}

#[test]
fn follows_the_omarchy_theme() {
    let dir = std::env::temp_dir().join(format!("obsidian-tui-theme-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    write_theme(&dir, "nord", "#2e3440");
    unsafe {
        std::env::set_var("OBSIDIAN_TUI_THEME_STATE", &dir);
    }
    let t = Theme::load();
    assert_eq!(t.name, "nord");
    assert!(!t.term_bg);
    assert_eq!(t.h("bg"), "#2e3440");

    let (v, _) = (PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/vault"), ());
    let mut app = obsidian_tui::app::App::with_state(&v, dir.join("state.json"));
    app.headless = true;
    std::thread::sleep(std::time::Duration::from_millis(20));
    write_theme(&dir, "tokyo-night", "#1a1b26");
    app.check_theme();
    assert_eq!(app.t.name, "tokyo-night");
    assert_eq!(app.t.h("bg"), "#1a1b26");

    // no Omarchy: the built-in palette on the terminal's own background
    unsafe {
        std::env::set_var("OBSIDIAN_TUI_THEME_STATE", dir.join("nowhere"));
    }
    let t = Theme::load();
    assert_eq!(t.name, "built-in");
    assert!(t.term_bg);
    assert_eq!(t.c("bg"), ratatui::style::Color::Reset);
}
