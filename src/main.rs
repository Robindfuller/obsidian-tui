//! obsidian-tui: an Obsidian vault in the terminal.
//!
//!     obsidian-tui [VAULT]           open the vault (the current folder if none)
//!     obsidian-tui --dump 160x45 [--steps "key:down;text:Garden"] [VAULT]
//!                                    print one screen as text and exit

use std::io::{self, Write};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use obsidian_tui::app::App;
use obsidian_tui::draw::draw;
use obsidian_tui::harness::H;
use obsidian_tui::util::editor;
use ratatui::crossterm::event::{
    self, DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture, Event,
};
use ratatui::crossterm::terminal::{
    Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen, SetTitle, disable_raw_mode, enable_raw_mode,
};
use ratatui::crossterm::{cursor, execute};
use ratatui::{Terminal, backend::CrosstermBackend};

const HELP: &str = "obsidian-tui: an Obsidian vault in the terminal

Usage:
  obsidian-tui [VAULT]      open a vault folder (the current folder if none)
  obsidian-tui --ask \"QUESTION\" [--model ID] [VAULT]
                            answer a question from the notes and exit

Keys: up/down or j/k move through notes, enter opens, / filters titles and
enter then searches every note, tab steps through links, [ and ] go back and
forward, a asks a question about your notes, e edits in $EDITOR, o opens in Obsidian, b folds the sidebar,
? lists every key, q quits. Everything is clickable.

It only reads the vault; editing happens in your own editor.";

fn restore() {
    let _ = disable_raw_mode();
    let _ = execute!(io::stdout(), DisableMouseCapture, DisableBracketedPaste, LeaveAlternateScreen, cursor::Show);
}

fn enter() -> io::Result<()> {
    enable_raw_mode()?;
    execute!(io::stdout(), EnterAlternateScreen, EnableMouseCapture, EnableBracketedPaste)
}

/// `--ask`: the notes it picked on stderr, the answer on stdout.
fn ask_once(vault: &std::path::Path, q: &str, model: Option<String>) -> i32 {
    use obsidian_tui::ask;
    let mut app = App::new(vault);
    let (models, why) = ask::all_models();
    let m = match model {
        Some(id) => {
            let id = if id.starts_with("ollama:") || id.starts_with("claude:") { id } else if id == "haiku" || id == "sonnet" || id == "opus" { format!("claude:{id}") } else { format!("ollama:{id}") };
            ask::Model { label: id.clone(), hint: String::new(), id }
        }
        None => {
            let saved = obsidian_tui::app::saved_model();
            match saved.and_then(|s| models.iter().find(|m| m.id == s).cloned()).or_else(|| models.first().cloned()) {
                Some(m) => m,
                None => {
                    eprintln!("obsidian-tui: nothing to answer with. {}", why.unwrap_or_default());
                    eprintln!("Start Ollama (ollama.com) or install Claude Code, or pass --model.");
                    return 1;
                }
            }
        }
    };
    app.chat.input.set(q);
    app.chat.ask(&app.vault, None, &m);
    let names = ask::names(&app.vault, &app.chat.turns[0].sources);
    let list: Vec<String> = names.into_iter().map(|(_, n)| n).collect();
    eprintln!("[{} · {}]", m.id, if list.is_empty() { "no notes matched".to_string() } else { list.join(", ") });
    let mut shown = 0;
    let mut out = io::stdout();
    loop {
        app.chat.poll();
        let t = &app.chat.turns[0];
        let a = t.answer();
        if a.len() > shown && a.is_char_boundary(shown) {
            let _ = out.write_all(&a.as_bytes()[shown..]);
            let _ = out.flush();
            shown = a.len();
        }
        if t.done {
            println!();
            if let Some(e) = &t.err {
                eprintln!("{e}");
                return 1;
            }
            return 0;
        }
        std::thread::sleep(Duration::from_millis(30));
    }
}

fn main() -> io::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "-h" || a == "--help") {
        println!("{HELP}");
        return Ok(());
    }
    if args.iter().any(|a| a == "-V" || a == "--version") {
        println!("obsidian-tui {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    // the vault is the first argument that isn't a flag or a flag's value
    let mut vault: Option<PathBuf> = None;
    let mut dump: Option<(u16, u16)> = None;
    let mut steps: Option<String> = None;
    let mut ask: Option<String> = None;
    let mut model: Option<String> = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--dump" => {
                let size = args.get(i + 1).cloned().unwrap_or("160x45".into());
                dump = Some(
                    size.split_once('x')
                        .map(|(w, h)| (w.parse().unwrap_or(160), h.parse().unwrap_or(45)))
                        .unwrap_or((160, 45)),
                );
                i += 2;
            }
            "--steps" => {
                steps = args.get(i + 1).cloned();
                i += 2;
            }
            "--ask" => {
                ask = args.get(i + 1).cloned();
                i += 2;
            }
            "--model" => {
                model = args.get(i + 1).cloned();
                i += 2;
            }
            a if a.starts_with("--") => {
                eprintln!("obsidian-tui: unknown option {a} (try --help)");
                std::process::exit(2);
            }
            a => {
                if vault.is_none() {
                    vault = Some(PathBuf::from(a));
                }
                i += 1;
            }
        }
    }
    let vault = match vault {
        Some(v) => v,
        None => std::env::current_dir()?,
    };
    if !vault.is_dir() {
        eprintln!("obsidian-tui: {} is not a folder. Give it the path to your vault:", vault.display());
        eprintln!("  obsidian-tui ~/Documents/MyVault");
        std::process::exit(2);
    }

    if let Some(q) = ask {
        std::process::exit(ask_once(&vault, &q, model));
    }

    if let Some((w, h)) = dump {
        let state = std::env::temp_dir().join(format!("obsidian-tui-dump-{}.json", std::process::id()));
        let mut hh = H::new(&vault, &state, w, h);
        if let Some(s) = &steps {
            hh.steps(s);
        }
        print!("{}", hh.render());
        let _ = std::fs::remove_file(state);
        return Ok(());
    }

    let mut app = App::new(&vault);
    let hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        restore();
        hook(info);
    }));
    enter()?;
    execute!(io::stdout(), SetTitle(format!("{} · Obsidian", app.vault.name)))?;
    let mut term = Terminal::new(CrosstermBackend::new(io::stdout()))?;
    term.hide_cursor()?;
    let sz = term.size()?;
    app.size = (sz.width, sz.height);
    app.start();
    let mut last_draw = Instant::now() - Duration::from_secs(1);
    while !app.quit {
        app.tick();
        if let Some(path) = app.pending_edit.take() {
            // hand the terminal to the editor, then take it back
            restore();
            let ed = editor();
            let status = std::process::Command::new(&ed[0]).args(&ed[1..]).arg(&path).status();
            enter()?;
            execute!(io::stdout(), Clear(ClearType::All))?;
            // a fresh Terminal repaints everything; clear() would ask the
            // terminal where its cursor is, and not every terminal answers
            term = Terminal::new(CrosstermBackend::new(io::stdout()))?;
            term.hide_cursor()?;
            if let Err(e) = status {
                app.toast("Couldn't start the editor", &format!("{}: {e}. Set $EDITOR to the one you use.", ed[0]), obsidian_tui::app::Sev::Error);
            }
            app.rescan_now();
            app.dirty = true;
        }
        if !app.pending_out.is_empty() {
            let mut out = io::stdout();
            for s in app.pending_out.drain(..) {
                let _ = out.write_all(s.as_bytes());
            }
            let _ = out.flush();
        }
        if app.dirty || last_draw.elapsed() > Duration::from_millis(1000) {
            term.draw(|f| draw(&mut app, f.buffer_mut()))?;
            app.dirty = false;
            last_draw = Instant::now();
        }
        if event::poll(Duration::from_millis(50))? {
            loop {
                match event::read()? {
                    Event::Key(k) => app.on_key(k),
                    Event::Mouse(m) => app.on_mouse(m),
                    Event::Paste(s) => app.on_paste(&s),
                    Event::Resize(w, h) => {
                        app.size = (w, h);
                        app.dirty = true;
                    }
                    _ => {}
                }
                if app.quit || app.pending_edit.is_some() || !event::poll(Duration::from_millis(0))? {
                    break;
                }
            }
        }
    }
    app.save_state();
    restore();
    io::stdout().flush()
}
