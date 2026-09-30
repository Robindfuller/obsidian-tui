# obsidian-tui

An Obsidian vault in the terminal, in the style of btop and herdr. A sidebar
of folders and tags, the list of notes, and the note itself rendered, with
every link, tag and folder clickable and a search across the whole vault.

Press `e` to edit a note right there, in a simple built-in editor (no modes:
just type, ctrl+s to save). `E` hands it to your own `$EDITOR` instead. The
app writes to your vault only when you save in that editor, and never touches
Obsidian's own `.obsidian` folder.

Runs on Linux, macOS and Windows (Windows Terminal).

## Install

Download the archive for your system from the
[latest release](https://github.com/Robindfuller/obsidian-tui/releases/latest),
unpack it and put `obsidian-tui` (`obsidian-tui.exe` on Windows) somewhere on
your PATH.

- **Windows:** `obsidian-tui-windows-x86_64.zip`. Run it in Windows Terminal:
  `obsidian-tui.exe C:\path\to\vault`. The first time, Windows may say it
  protected your PC because the app isn't signed: click **More info**, then
  **Run anyway**.
- **macOS:** `obsidian-tui-macos-arm64.tar.gz` (Apple silicon) or
  `obsidian-tui-macos-x86_64.tar.gz` (Intel). If macOS won't open it, clear
  the download flag: `xattr -d com.apple.quarantine obsidian-tui`
- **Linux:** `obsidian-tui-linux-x86_64.tar.gz`.

Or build it yourself with Rust (https://rustup.rs):

```sh
cargo install --git https://github.com/Robindfuller/obsidian-tui
```

## Use

```sh
obsidian-tui ~/Documents/MyVault
obsidian-tui "C:\Users\me\Documents\My Vault"
obsidian-tui            # the current folder
```

What's on screen:

- **Sidebar:** All notes, Recent, Graph, the folder tree (click ▸ to open a folder
  without leaving the list), and every #tag. Nested tags take in their
  children: `#projects` shows `#projects/garden` too.
- **Note list:** the notes in whatever the sidebar has chosen, a to z or newest
  first (`s`, or click ↕). Type to filter titles; press enter to search inside
  every note instead.
- **The note:** headings, bold, italic, strikethrough, ==highlights==, lists
  and tasks, quotes and callouts (`> [!tip]`), code, tables, rules, and
  frontmatter shown neatly at the top. `[[wikilinks]]` (with `|aliases` and
  `#headings`), Markdown links and `#tags` are all clickable; links to notes
  that don't exist are dimmed. `![[embeds]]` show as a line you can click to
  open the file. `%%comments%%` are hidden.
- **Tabs:** Note, Links (backlinks with the line that links here, and every
  link out), Outline (click a heading to jump to it) and Graph.

## Graph

Like Obsidian's graph view, drawn with braille dots and lines.

- **The note's graph** (`g`, or the Graph tab): the open note in the middle
  (◉), with what it links to and what links to it around it. `+` and `-`
  reach out 1, 2 or 3 links. Links to notes that don't exist yet are hollow
  dimmer dots (○).
- **The vault's graph** (Graph in the sidebar): every note, filling the space
  beside the sidebar.

Dots are coloured by top-level folder. Hover a dot to pick it out with its
labels and lines; click a dot or its name to open the note. The arrow keys
move between nearby dots and enter opens one. The wheel zooms (on the spot
under the mouse), dragging empty space pans, `+` `-` zoom the vault's graph,
`0` resets the view and esc goes back.

The layout is worked out the same way every time, so a vault always gives the
same picture. It's worked out once and kept until the vault changes: a
3,000-note vault takes about half a second the first time.

Links resolve the way Obsidian does: a path from the vault root or from the
note's own folder, then a note of that name (the one nearest the linking note
wins when two share a name), then an alias from frontmatter.

The vault is watched: notes you add, change or delete elsewhere show up at
once.

## Editing

`e` opens the note in a built-in editor in the note's place (the whole window
when it's narrow). There are no modes, like nano:

| Key | Does |
| --- | --- |
| type, enter, backspace, delete | what you'd expect |
| arrows, home, end, pgup, pgdn | move (home again goes to the very start of the line) |
| ctrl+arrows, ctrl+home/end | a word at a time, the start or end of the note |
| shift + any of those, or drag | select |
| click | put the cursor there; the wheel scrolls |
| ctrl+c, ctrl+x, ctrl+v | copy, cut, paste (the whole line if nothing's selected) |
| ctrl+a | select everything |
| ctrl+z, ctrl+y (or ctrl+shift+z) | undo, redo |
| ctrl+f | find; enter for the next match, shift+enter the one before, esc to close |
| tab, shift+tab | indent the way the file does (tabs or spaces), outdent |
| ctrl+s | save |
| esc | done; asks Save / Discard / Keep editing if there's anything unsaved |

Long lines wrap on screen, never in the file. Headings, links, tags and code
are coloured. Copy and paste use the system clipboard where there is one
(Wayland, X11, macOS, Windows), and pasting from the terminal works too.

Saving is careful. The file keeps its line endings (Windows CRLF stays CRLF),
its byte-order mark and whether it ends with a newline. The text goes to a
hidden temp file beside the note, which is then renamed over it, so a crash
can't leave half a note. If the file changed on disk after you opened it
(Obsidian or sync got there first) it asks whether to save yours over it, load
theirs, or keep editing. Files that aren't UTF-8 text are left alone: use `E`.

## Keys

| Key | Does |
| --- | --- |
| ↑ ↓ or j k | move through the notes (always, whatever you last clicked) |
| enter | open the note, or follow the link picked with tab |
| tab, shift+tab | step through the links in the note |
| / | filter titles as you type; enter searches every note |
| n, N | next / previous search match in the note |
| [ ], alt+← → | back / forward |
| ← or h | move into the sidebar (↑ ↓ there, → or esc to come back) |
| 1 2 3 4 | Note, Links, Outline, Graph |
| g | the note's graph (again for the note) |
| + -, 0 | in a graph: more or fewer links (zoom in the vault's), reset the view |
| space, pgup, pgdn, ctrl+d, ctrl+u | scroll the note |
| J K, home, end | scroll a line, or to the top or bottom |
| e | edit the note here (see Editing) |
| E | edit in `$VISUAL` / `$EDITOR` (notepad on Windows, else nano or vi) |
| o | open the note in the Obsidian app |
| y, Y | copy `[[link]]` / the file's path (over OSC 52) |
| s | sort by name or by date |
| b, ctrl+b | fold the sidebar |
| { } | narrower / wider note list (or drag its edge) |
| r | read the vault again |
| ? | all of this, in the app |
| q, ctrl+c | quit |

Drag the edge between the note list and the note to make the list wider or
narrower (the note always keeps at least 40 columns); it's remembered.

Right-click a note in the list for a menu.

In a narrow terminal (under 100 columns) it shows one pane at a time and you
step through them: the sidebar (pick a folder or tag), then its notes, then the
note, each the full width. Enter, → or a click goes forward; esc, ← or the ‹ in
the title goes back. In the sidebar, ↑ ↓ move and ← → fold folders; in the
note, ↑ ↓ scroll it. Widen the window and all three come back side by side.

## Look

On Omarchy it uses the live Omarchy theme (the same colour maths as Monday
Board and Scribe) and follows theme switches. Anywhere else it uses a built-in
dark palette on your terminal's own background, or a light one if the terminal
says it's light (`COLORFGBG`); force either with `OBSIDIAN_TUI_THEME=light` or
`dark`. It needs a terminal with true colour and mouse support: Windows
Terminal, iTerm2, kitty, WezTerm, foot, ghostty, alacritty, GNOME Terminal
and so on.

It remembers the sidebar, sort, list width, open folders and the last note per
vault in `~/.config/obsidian-tui/state.json` (`%APPDATA%\obsidian-tui` on
Windows).

## Develop

```sh
cargo test                     # screens, clicks, links, search, watching, editing
cargo clippy --all-targets
UPDATE_GOLDEN=1 cargo test     # rewrite the saved screens in tests/golden
obsidian-tui --dump 150x40 --steps "text:Welcome;key:tab,enter" tests/fixtures/vault
```

`--dump` prints one screen as text and exits; `--steps` presses keys
(`key:down,enter`), types (`type:compost`) and clicks (`click:60,4`,
`text:Garden Plan`) first. The tests run against the made-up vault in
`tests/fixtures/vault`, copied to a temp folder so nothing real is touched.

## Licence

MIT, see [LICENSE](LICENSE).
