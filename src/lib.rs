//! An Obsidian vault in the terminal, in the style of btop and herdr: a
//! sidebar of folders and tags, a list of notes, the note itself rendered,
//! every link and tag clickable, and search across the whole vault.

pub mod app;
pub mod draw;
pub mod editor;
pub mod events;
pub mod harness;
pub mod input;
pub mod md;
pub mod rich;
pub mod theme;
pub mod util;
pub mod vault;
