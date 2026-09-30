//! Colours. On Omarchy this is the live theme, derived exactly the way Monday
//! Board and Scribe do it (`derive` and friends are a line-for-line port of
//! their theme.py, rounding half to even like Python) and it follows theme
//! switches. Anywhere else it's a built-in dark or light palette on the
//! terminal's own background.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use ratatui::style::Color;

pub const TARGET: f64 = 4.5;
pub const LARGE: f64 = 3.0;
pub const STEP: f64 = 1.28;


// ------------------------------------------------------------- colour maths
pub fn hex(c: &str) -> (i64, i64, i64) {
    let c = c.trim().trim_start_matches('#');
    let c: String = if c.chars().count() == 3 { c.chars().flat_map(|ch| [ch, ch]).collect() } else { c.to_string() };
    let p = |i: usize| c.get(i..i + 2).and_then(|s| i64::from_str_radix(s, 16).ok()).unwrap_or(0);
    (p(0), p(2), p(4))
}

fn s_rgb(r: f64, g: f64, b: f64) -> String {
    let f = |v: f64| v.round_ties_even().clamp(0.0, 255.0) as i64;
    format!("#{:02x}{:02x}{:02x}", f(r), f(g), f(b))
}

pub fn luminance(c: &str) -> f64 {
    let f = |v: i64| {
        let v = v as f64 / 255.0;
        if v <= 0.03928 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
    };
    let (r, g, b) = hex(c);
    0.2126 * f(r) + 0.7152 * f(g) + 0.0722 * f(b)
}

pub fn contrast(a: &str, b: &str) -> f64 {
    let (la, lb) = (luminance(a), luminance(b));
    (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
}

/// theme.py's mix: t of the way from a to b.
pub fn tmix(a: &str, b: &str, t: f64) -> String {
    let (ra, rb) = (hex(a), hex(b));
    let m = |x: i64, y: i64| x as f64 + (y as f64 - x as f64) * t;
    s_rgb(m(ra.0, rb.0), m(ra.1, rb.1), m(ra.2, rb.2))
}

/// The TUI's mix (omtheme._mix and board_tui.mix): t of colour a, the rest b.
pub fn mix(a: &str, b: &str, t: f64) -> String {
    let (ra, rb) = (hex(a), hex(b));
    let m = |x: i64, y: i64| x as f64 * t + y as f64 * (1.0 - t);
    s_rgb(m(ra.0, rb.0), m(ra.1, rb.1), m(ra.2, rb.2))
}

fn first_min<F: Fn(&str) -> f64>(xs: &[String], key: F) -> &str {
    let mut best = &xs[0];
    let mut bv = key(best);
    for x in &xs[1..] {
        let v = key(x);
        if v < bv {
            best = x;
            bv = v;
        }
    }
    best
}

fn worst(fg: &str, surfaces: &[String]) -> f64 {
    surfaces.iter().map(|s| contrast(fg, s)).fold(f64::INFINITY, f64::min)
}

pub fn ensure_contrast(fg: &str, surfaces: &[String], target: f64) -> String {
    if surfaces.is_empty() {
        return fg.to_string();
    }
    let w = first_min(surfaces, |s| contrast(fg, s)).to_string();
    if contrast(fg, &w) >= target {
        return fg.to_string();
    }
    let pole = if luminance(&w) < 0.5 { "#ffffff" } else { "#000000" };
    let (mut best, mut best_ratio) = (fg.to_string(), contrast(fg, &w));
    for i in 1..=20 {
        let cand = tmix(fg, pole, i as f64 / 20.0);
        let r = worst(&cand, surfaces);
        if r > best_ratio {
            best = cand.clone();
            best_ratio = r;
        }
        if r >= target {
            return cand;
        }
    }
    best
}

fn spread(colours: &[String], surfaces: &[String]) -> Vec<String> {
    let darkest = first_min(surfaces, luminance);
    let pole = if luminance(darkest) < 0.5 { "#ffffff" } else { "#000000" };
    let ceiling = worst(pole, surfaces);
    let mut need = TARGET;
    let mut out = vec![];
    for c in colours {
        let c = ensure_contrast(c, surfaces, need.min(ceiling));
        need = worst(&c, surfaces) * STEP;
        out.push(c);
    }
    out
}

/// theme.py's _hsl: (degrees, saturation, lightness).
fn hsl(c: &str) -> (f64, f64, f64) {
    let (r, g, b) = hex(c);
    let (r, g, b) = (r as f64 / 255.0, g as f64 / 255.0, b as f64 / 255.0);
    let (hi, lo) = (r.max(g).max(b), r.min(g).min(b));
    let (l, d) = ((hi + lo) / 2.0, hi - lo);
    if d == 0.0 {
        return (0.0, 0.0, l);
    }
    let sat = if l > 0.5 { d / (2.0 - hi - lo) } else { d / (hi + lo) };
    let h = if hi == r {
        ((g - b) / d).rem_euclid(6.0)
    } else if hi == g {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    };
    (h * 60.0, sat, l)
}

fn unhsl(h: f64, sat: f64, l: f64) -> String {
    let h = h.rem_euclid(360.0);
    let c = (1.0 - (2.0 * l - 1.0).abs()) * sat;
    let x = c * (1.0 - ((h / 60.0).rem_euclid(2.0) - 1.0).abs());
    let (r, g, b) = [(c, x, 0.0), (x, c, 0.0), (0.0, c, x), (0.0, x, c), (x, 0.0, c), (c, 0.0, x)]
        [((h / 60.0).floor() as usize).min(5)];
    let m = l - c / 2.0;
    s_rgb((r + m) * 255.0, (g + m) * 255.0, (b + m) * 255.0)
}

fn same(a: &str, b: &str) -> bool {
    let (ha, sa, la) = hsl(a);
    let (hb, sb, lb) = hsl(b);
    if sa < 0.15 && sb < 0.15 {
        return (la - lb).abs() < 0.12;
    }
    let dh = (ha - hb).abs();
    dh.min(360.0 - dh) < 22.0 && (la - lb).abs() < 0.12
}

fn distinguish(colours: &[String], surfaces: &[String]) -> Vec<String> {
    let mut out: Vec<String> = vec![];
    for c in colours {
        let mut c = ensure_contrast(c, surfaces, TARGET);
        for turn in 1..12 {
            if !out.iter().any(|p| same(&c, p)) {
                break;
            }
            let (h, sat, l) = hsl(&c);
            c = ensure_contrast(&unhsl(h + turn as f64 * 47.0, sat.max(0.32), l), surfaces, TARGET);
        }
        out.push(c);
    }
    out
}

// ------------------------------------------------------------- palette read
type Raw = HashMap<String, String>;

fn is_hexish(v: &str) -> bool {
    (3..=6).contains(&v.len()) && v.chars().all(|c| c.is_ascii_hexdigit())
}

fn from_alacritty(path: &Path) -> Raw {
    let Ok(text) = std::fs::read_to_string(path) else { return Raw::new() };
    let Ok(d) = text.parse::<toml::Table>() else { return Raw::new() };
    let keys = [
        ("background", "colors.primary", "background"),
        ("foreground", "colors.primary", "foreground"),
        ("red", "colors.normal", "red"),
        ("green", "colors.normal", "green"),
        ("yellow", "colors.normal", "yellow"),
        ("blue", "colors.normal", "blue"),
        ("magenta", "colors.normal", "magenta"),
        ("cyan", "colors.normal", "cyan"),
        ("selection", "colors.selection", "background"),
    ];
    let mut out = Raw::new();
    for (key, sect, name) in keys {
        let mut node: Option<&toml::Table> = Some(&d);
        for part in sect.split('.') {
            node = node.and_then(|n| n.get(part)).and_then(|v| v.as_table());
        }
        if let Some(v) = node.and_then(|n| n.get(name)).and_then(|v| v.as_str()) {
            // the regex is #?[0-9a-fA-F]{3,6}, fullmatch on the stripped value
            let t = v.trim();
            let body = t.strip_prefix('#').unwrap_or(t);
            if is_hexish(body) {
                out.insert(key.into(), format!("#{body}"));
            }
        }
    }
    if out.contains_key("background") && out.contains_key("foreground") {
        let acc = out.get("yellow").or(out.get("foreground")).cloned().unwrap_or_default();
        out.entry("accent".into()).or_insert(acc);
        let mode = if luminance(&out["background"]) < 0.5 { "dark" } else { "light" };
        out.insert("mode".into(), mode.into());
    }
    out
}

pub fn read_palette(theme_dir: &Path) -> Raw {
    let mut raw = Raw::new();
    if let Ok(text) = std::fs::read_to_string(theme_dir.join("colors.toml"))
        && let Ok(t) = text.parse::<toml::Table>()
    {
        for (k, v) in t {
            if let Some(s) = v.as_str() {
                raw.insert(k, s.to_string());
            }
        }
    }
    if raw.get("background").is_none_or(|s| s.is_empty()) {
        let mut merged = from_alacritty(&theme_dir.join("alacritty.toml"));
        merged.extend(raw);
        raw = merged;
    }
    for (src, dst) in [
        ("color1", "red"),
        ("color2", "green"),
        ("color3", "yellow"),
        ("color4", "blue"),
        ("color5", "magenta"),
        ("color6", "cyan"),
        ("selection_background", "selection"),
    ] {
        if let Some(v) = raw.get(src).cloned() {
            raw.entry(dst.into()).or_insert(v);
        }
    }
    raw
}

// ------------------------------------------------------------- derivation
fn get<'a>(raw: &'a Raw, k: &str) -> Option<&'a str> {
    raw.get(k).map(|s| s.as_str()).filter(|s| !s.is_empty())
}

fn quieter(candidate: Option<&str>, ink: &str, bg: &str, t: f64) -> String {
    match candidate {
        Some(c) if contrast(c, bg) < contrast(ink, bg) => c.to_string(),
        _ => tmix(ink, bg, t),
    }
}

fn hued(candidate: Option<&str>, hue: f64, base: &str, surfaces: &[String], floor: f64) -> String {
    if let Some(c) = candidate {
        return ensure_contrast(c, surfaces, floor);
    }
    let (_, sat, l) = hsl(base);
    ensure_contrast(&unhsl(hue, sat.max(0.45), l), surfaces, floor)
}

/// Raw Omarchy palette -> the app's semantic tokens (without the leading --).
pub fn derive(raw: &Raw) -> (String, HashMap<String, String>) {
    let bg = get(raw, "background").unwrap_or("#05182e").to_string();
    let fg = get(raw, "foreground").unwrap_or("#f6dcac").to_string();
    let mode = get(raw, "mode")
        .map(str::to_string)
        .unwrap_or_else(|| if luminance(&bg) < 0.5 { "dark".into() } else { "light".into() });
    let dark = mode == "dark";

    let mut page = get(raw, "dark_background").map(str::to_string).unwrap_or_else(|| tmix(&bg, &fg, 0.02));
    let panel = bg.clone();
    let mut raised = get(raw, "lighter_background").map(str::to_string).unwrap_or_else(|| tmix(&bg, &fg, 0.07));
    // Python precedence: (darker_background or mix(bg, fg, 0.0)) if dark else mix(bg, fg, 0.04)
    let mut nav = if dark {
        get(raw, "darker_background").map(str::to_string).unwrap_or_else(|| tmix(&bg, &fg, 0.0))
    } else {
        tmix(&bg, &fg, 0.04)
    };
    if !dark {
        page = tmix(&bg, &fg, 0.03);
        raised = tmix(&bg, &fg, 0.08);
        nav = tmix(&bg, &fg, 0.06);
    }
    let surfaces = vec![page.clone(), panel.clone(), raised.clone(), nav.clone()];

    let accent0 = get(raw, "accent").or(get(raw, "orange")).or(get(raw, "yellow")).unwrap_or(&fg).to_string();
    let ink0 = fg.clone();
    let dim0 = quieter(get(raw, "light_foreground"), &ink0, &bg, 0.25);
    let sea0 = quieter(get(raw, "dark_foreground").or(get(raw, "blue")), &ink0, &bg, 0.5);
    let faint = get(raw, "muted").map(str::to_string).unwrap_or_else(|| tmix(&fg, &bg, 0.68));
    let line = tmix(&bg, &fg, 0.14);
    let line_soft = tmix(&bg, &fg, 0.08);
    let sel = get(raw, "selection").map(str::to_string).unwrap_or_else(|| tmix(&bg, &accent0, 0.25));

    let ramp = vec![
        accent0.clone(),
        get(raw, "blue").map(str::to_string).unwrap_or(sea0.clone()),
        get(raw, "cyan").map(str::to_string).unwrap_or(dim0.clone()),
        get(raw, "yellow").or(get(raw, "orange")).map(str::to_string).unwrap_or(accent0.clone()),
        get(raw, "green").or(get(raw, "teal")).map(str::to_string).unwrap_or(sea0.clone()),
    ];

    let sp = spread(&[sea0, dim0, accent0, ink0], &surfaces);
    let (sea, dim, accent, ink) = (sp[0].clone(), sp[1].clone(), sp[2].clone(), sp[3].clone());

    let mut t = HashMap::new();
    let mut put = |k: &str, v: String| {
        t.insert(k.to_string(), v);
    };
    put("nav", nav);
    put("bg", page);
    put("panel", panel);
    put("raise", raised);
    put("line", line);
    put("line-soft", line_soft);
    put("sel", sel);
    put("ink", ink);
    put("dim", dim.clone());
    put("sea", sea.clone());
    put("faint", faint);
    put("accent", accent.clone());
    put("amber", hued(get(raw, "yellow"), 42.0, &accent, &surfaces, LARGE));
    put("flame", hued(get(raw, "red"), 8.0, &accent, &surfaces, LARGE));
    put("teal", hued(get(raw, "green"), 148.0, &sea, &surfaces, LARGE));
    put("cyan", ensure_contrast(get(raw, "cyan").unwrap_or(&dim), &surfaces, TARGET));
    for (i, c) in distinguish(&ramp, &surfaces).into_iter().enumerate() {
        put(&format!("s{}", i + 1), c);
    }
    (mode, t)
}

fn raw(pairs: &[(&str, &str)]) -> Raw {
    pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
}

/// For terminals that aren't on Omarchy: a quiet dark palette...
fn builtin_dark() -> Raw {
    raw(&[
        ("background", "#16181d"),
        ("foreground", "#d4d7dc"),
        ("accent", "#7aa2f7"),
        ("red", "#e06c75"),
        ("green", "#98c379"),
        ("yellow", "#e5c07b"),
        ("blue", "#61afef"),
        ("magenta", "#c678dd"),
        ("cyan", "#56b6c2"),
        ("selection", "#2c313c"),
        ("mode", "dark"),
    ])
}

/// ...and a light one, for terminals with a light background.
fn builtin_light() -> Raw {
    raw(&[
        ("background", "#fafafa"),
        ("foreground", "#383a42"),
        ("accent", "#4078f2"),
        ("red", "#e45649"),
        ("green", "#50a14f"),
        ("yellow", "#c18401"),
        ("blue", "#4078f2"),
        ("magenta", "#a626a4"),
        ("cyan", "#0184bc"),
        ("selection", "#dfe3ea"),
        ("mode", "light"),
    ])
}

/// Is the terminal light? OBSIDIAN_TUI_THEME=light|dark wins, then COLORFGBG
/// ("15;0" is light text on a dark background), else assume dark.
fn light_terminal() -> bool {
    match std::env::var("OBSIDIAN_TUI_THEME").as_deref() {
        Ok("light") => return true,
        Ok("dark") => return false,
        _ => {}
    }
    std::env::var("COLORFGBG")
        .ok()
        .and_then(|v| v.rsplit(';').next().and_then(|b| b.trim().parse::<u8>().ok()))
        .is_some_and(|bg| bg == 7 || bg >= 9)
}

// ------------------------------------------------------------- the TUI's theme
pub fn state_dir() -> PathBuf {
    if let Ok(d) = std::env::var("OBSIDIAN_TUI_THEME_STATE") {
        return PathBuf::from(d);
    }
    crate::util::home().join(".local/state/omarchy/current")
}

/// Changes whenever the theme does (omarchy-theme-set swaps the dir).
pub fn stamp() -> Vec<u128> {
    let st = state_dir();
    [st.join("theme.name"), st.join("theme").join("colors.toml"), st.join("theme")]
        .iter()
        .map(|p| {
            std::fs::metadata(p)
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        })
        .collect()
}

pub struct Theme {
    pub tok: HashMap<String, String>,
    pub name: String,
    pub mode: String,
    /// Not on Omarchy: leave the terminal's own background showing.
    pub term_bg: bool,
}

impl Theme {
    pub fn load() -> Theme {
        let st = state_dir();
        let mut raw = read_palette(&st.join("theme"));
        let omarchy = raw.get("background").is_some_and(|s| !s.is_empty());
        if !omarchy {
            raw = if light_terminal() { builtin_light() } else { builtin_dark() };
        }
        let (_, tok) = derive(&raw);
        let name = if omarchy {
            std::fs::read_to_string(st.join("theme.name")).map(|s| s.trim().to_string()).unwrap_or("omarchy".into())
        } else {
            "built-in".into()
        };
        // Omarchy's terminals already paint the theme's background, which is
        // Board's --panel. Sit on that and step up from it for hover and selection.
        let mut t = tok.clone();
        t.insert("bg".into(), tok["panel"].clone());
        t.insert("panel".into(), tok["raise"].clone());
        t.insert("raise".into(), tok["sel"].clone());
        t.insert("sel".into(), mix(&tok["ink"], &tok["sel"], 0.14));
        Theme { tok: t, name, mode: raw.get("mode").cloned().unwrap_or("dark".into()), term_bg: !omarchy }
    }

    /// A token's hex value ("ink", "faint", ...).
    pub fn h(&self, k: &str) -> &str {
        self.tok.get(k).map(|s| s.as_str()).unwrap_or("#888888")
    }

    pub fn c(&self, k: &str) -> Color {
        if k == "bg" && self.term_bg {
            return Color::Reset;
        }
        color(self.h(k))
    }

    /// A token's colour blended toward the ink, for quieter text in a hue.
    pub fn soft(&self, tok: &str, t: f64) -> Color {
        color(&mix(self.h(tok), self.h("ink"), t))
    }
}

pub fn color(h: &str) -> Color {
    let (r, g, b) = hex(h);
    Color::Rgb(r as u8, g as u8, b as u8)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rounds_half_to_even() {
        // 0x10 * .5 + 0x11 * .5 = 16.5 -> 16 in Python
        assert_eq!(mix("#101010", "#111111", 0.5), "#101010");
    }

    #[test]
    fn builtins_derive_readably() {
        for raw in [builtin_dark(), builtin_light()] {
            let (_, t) = derive(&raw);
            assert!(t.contains_key("s5"));
            assert!(contrast(&t["ink"], &t["bg"]) >= TARGET);
        }
    }
}
