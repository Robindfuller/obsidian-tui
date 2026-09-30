//! The graph: notes as dots, links as lines. `local` is the open note with
//! what it links to and what links to it, out to a few hops; `whole` is the
//! vault. Both are laid out with a force-directed layout (links pull, dots
//! push apart, a little gravity keeps islands close) that starts from a fixed
//! spiral and uses no randomness, so a vault always gives the same picture.
//! Repulsion only looks at nearby dots through a grid, and big graphs get
//! fewer rounds, so thousands of notes still lay out in a blink.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use crate::vault::Vault;

#[derive(Clone, Debug, PartialEq)]
pub struct GNode {
    /// a note key, or "?name" for a link to a note that doesn't exist
    pub id: String,
    pub label: String,
    /// the top-level folder, "" at the root
    pub folder: String,
    pub missing: bool,
    pub degree: usize,
    pub x: f64,
    pub y: f64,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Graph {
    pub nodes: Vec<GNode>,
    pub edges: Vec<(usize, usize)>,
    /// the note in the middle, for a local graph
    pub centre: Option<usize>,
    /// hops reached, for a local graph
    pub depth: usize,
    /// true if the local graph was cut short at the node limit
    pub capped: bool,
}

/// The ideal distance between linked dots.
const K: f64 = 10.0;
/// A local graph stops growing here, so three hops stay readable.
pub const LOCAL_MAX: usize = 400;

fn top_folder(key: &str) -> String {
    key.split_once('/').map(|(f, _)| f.to_string()).unwrap_or_default()
}

/// Where each note's links go: (resolved note keys, unresolved names), sorted.
fn links_of(v: &Vault, key: &str) -> (Vec<String>, Vec<String>) {
    let Some(n) = v.note(key) else { return (vec![], vec![]) };
    let mut out = BTreeSet::new();
    let mut missing = BTreeSet::new();
    for l in &n.links {
        if l.target.is_empty() {
            continue;
        }
        match v.resolve(&l.target, Some(key)) {
            Some(i) => {
                if v.notes[i].key != key {
                    out.insert(v.notes[i].key.clone());
                }
            }
            None => {
                // attachments aren't notes: leave them out
                if v.resolve_file(&l.target, Some(key)).is_none() {
                    missing.insert(l.target.clone());
                }
            }
        }
    }
    (out.into_iter().collect(), missing.into_iter().collect())
}

impl Graph {
    fn build(v: &Vault, ids: Vec<String>, with_missing: bool) -> Graph {
        let index: HashMap<&str, usize> = ids.iter().enumerate().map(|(i, s)| (s.as_str(), i)).collect();
        let mut edges = BTreeSet::new();
        for (i, id) in ids.iter().enumerate() {
            if id.starts_with('?') {
                continue;
            }
            let (out, missing) = links_of(v, id);
            for t in out {
                if let Some(&j) = index.get(t.as_str()) {
                    edges.insert((i.min(j), i.max(j)));
                }
            }
            if with_missing {
                for m in missing {
                    if let Some(&j) = index.get(format!("?{m}").as_str()) {
                        edges.insert((i.min(j), i.max(j)));
                    }
                }
            }
        }
        let mut nodes: Vec<GNode> = ids
            .iter()
            .map(|id| {
                let missing = id.starts_with('?');
                let label = if missing {
                    id[1..].rsplit('/').next().unwrap_or(&id[1..]).to_string()
                } else {
                    v.note(id).map(|n| n.name.clone()).unwrap_or_else(|| id.clone())
                };
                GNode {
                    id: id.clone(),
                    label,
                    folder: if missing { String::new() } else { top_folder(id) },
                    missing,
                    degree: 0,
                    x: 0.0,
                    y: 0.0,
                }
            })
            .collect();
        let edges: Vec<(usize, usize)> = edges.into_iter().collect();
        for &(a, b) in &edges {
            nodes[a].degree += 1;
            nodes[b].degree += 1;
        }
        Graph { nodes, edges, centre: None, depth: 0, capped: false }
    }

    /// Every note in the vault, and the links between them.
    pub fn whole(v: &Vault) -> Graph {
        let ids: Vec<String> = v.notes.iter().map(|n| n.key.clone()).collect();
        let mut g = Graph::build(v, ids, false);
        g.layout(None);
        g
    }

    /// The note, and everything within `depth` hops of it either way.
    pub fn local(v: &Vault, key: &str, depth: usize) -> Graph {
        let depth = depth.clamp(1, 3);
        // breadth first, in sorted order at each step so it's always the same
        let mut seen: BTreeMap<String, usize> = BTreeMap::new();
        let mut order: Vec<String> = vec![key.to_string()];
        seen.insert(key.to_string(), 0);
        let mut frontier = vec![key.to_string()];
        let mut capped = false;
        let mut reached = 0;
        'grow: for hop in 1..=depth {
            let mut next = BTreeSet::new();
            for k in &frontier {
                if k.starts_with('?') {
                    continue;
                }
                let (out, missing) = links_of(v, k);
                next.extend(out);
                next.extend(missing.into_iter().map(|m| format!("?{m}")));
                if let Some(back) = v.backlinks.get(k) {
                    next.extend(back.iter().map(|(s, _)| s.clone()));
                }
            }
            let mut added = vec![];
            for n in next {
                if seen.contains_key(&n) {
                    continue;
                }
                if order.len() >= LOCAL_MAX {
                    capped = true;
                    break 'grow;
                }
                seen.insert(n.clone(), hop);
                order.push(n.clone());
                added.push(n);
            }
            if added.is_empty() {
                break;
            }
            reached = hop;
            frontier = added;
        }
        let mut g = Graph::build(v, order, true);
        g.centre = Some(0);
        g.depth = reached.max(1);
        g.capped = capped;
        g.layout(Some(0));
        g
    }

    /// Fruchterman-Reingold, with a grid for the pushing apart.
    fn layout(&mut self, pin: Option<usize>) {
        let n = self.nodes.len();
        if n == 0 {
            return;
        }
        // a sunflower spiral to start from: spread out, and no randomness
        for (i, nd) in self.nodes.iter_mut().enumerate() {
            let r = K * 0.9 * (i as f64).sqrt();
            let a = i as f64 * 2.399_963_229_728_653;
            nd.x = r * a.cos();
            nd.y = r * a.sin();
        }
        if n == 1 {
            return;
        }
        let iters = match n {
            0..=150 => 300,
            151..=600 => 150,
            601..=1500 => 80,
            _ => 50,
        };
        let cutoff = K * 3.0;
        let all_pairs = n <= 150;
        let mut t = K * (n as f64).sqrt() * 0.4;
        let cool = t / (iters as f64 + 1.0);
        let mut disp = vec![(0.0f64, 0.0f64); n];
        let cell = |x: f64, y: f64| ((x / cutoff).floor() as i64, (y / cutoff).floor() as i64);
        for _ in 0..iters {
            for d in disp.iter_mut() {
                *d = (0.0, 0.0);
            }
            // push apart
            if all_pairs {
                for i in 0..n {
                    for j in i + 1..n {
                        let (dx, dy) = (self.nodes[i].x - self.nodes[j].x, self.nodes[i].y - self.nodes[j].y);
                        let d2 = (dx * dx + dy * dy).max(0.01);
                        if d2 > cutoff * cutoff {
                            continue;
                        }
                        let f = K * K / d2;
                        disp[i].0 += dx * f;
                        disp[i].1 += dy * f;
                        disp[j].0 -= dx * f;
                        disp[j].1 -= dy * f;
                    }
                }
            } else {
                let mut grid: HashMap<(i64, i64), Vec<usize>> = HashMap::new();
                for (i, nd) in self.nodes.iter().enumerate() {
                    grid.entry(cell(nd.x, nd.y)).or_default().push(i);
                }
                for (i, d) in disp.iter_mut().enumerate() {
                    let (cx, cy) = cell(self.nodes[i].x, self.nodes[i].y);
                    for gx in cx - 1..=cx + 1 {
                        for gy in cy - 1..=cy + 1 {
                            let Some(b) = grid.get(&(gx, gy)) else { continue };
                            for &j in b {
                                if j == i {
                                    continue;
                                }
                                let (dx, dy) = (self.nodes[i].x - self.nodes[j].x, self.nodes[i].y - self.nodes[j].y);
                                let d2 = (dx * dx + dy * dy).max(0.01);
                                if d2 > cutoff * cutoff {
                                    continue;
                                }
                                let f = K * K / d2;
                                d.0 += dx * f;
                                d.1 += dy * f;
                            }
                        }
                    }
                }
            }
            // links pull together
            for &(a, b) in &self.edges {
                let (dx, dy) = (self.nodes[a].x - self.nodes[b].x, self.nodes[a].y - self.nodes[b].y);
                let d = (dx * dx + dy * dy).sqrt().max(0.01);
                let f = d / K;
                disp[a].0 -= dx * f;
                disp[a].1 -= dy * f;
                disp[b].0 += dx * f;
                disp[b].1 += dy * f;
            }
            // gravity keeps islands and loose notes near the middle
            for (i, nd) in self.nodes.iter().enumerate() {
                let g = if nd.degree == 0 { 0.6 } else { 0.1 };
                disp[i].0 -= nd.x * g;
                disp[i].1 -= nd.y * g;
            }
            for (i, nd) in self.nodes.iter_mut().enumerate() {
                if Some(i) == pin {
                    continue;
                }
                let (dx, dy) = disp[i];
                let d = (dx * dx + dy * dy).sqrt();
                if d > 0.0 {
                    let s = d.min(t) / d;
                    nd.x += dx * s;
                    nd.y += dy * s;
                }
            }
            t = (t - cool).max(K * 0.02);
        }
        if let Some(p) = pin {
            let (px, py) = (self.nodes[p].x, self.nodes[p].y);
            for nd in self.nodes.iter_mut() {
                nd.x -= px;
                nd.y -= py;
            }
        }
    }

    /// (min x, min y, max x, max y) of the dots.
    pub fn bounds(&self) -> (f64, f64, f64, f64) {
        let mut b = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
        for n in &self.nodes {
            b = (b.0.min(n.x), b.1.min(n.y), b.2.max(n.x), b.3.max(n.y));
        }
        if self.nodes.is_empty() { (0.0, 0.0, 0.0, 0.0) } else { b }
    }

    pub fn find(&self, id: &str) -> Option<usize> {
        self.nodes.iter().position(|n| n.id == id)
    }
}

/// Zoom and pan: the view on top of "fit everything".
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct View {
    pub zoom: f64,
    pub pan_x: f64,
    pub pan_y: f64,
}

impl Default for View {
    fn default() -> View {
        View { zoom: 1.0, pan_x: 0.0, pan_y: 0.0 }
    }
}

/// How graph coordinates land on screen at the last draw.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Xform {
    /// the middle of the drawing area, in cells
    pub mid_x: f64,
    pub mid_y: f64,
    /// cells per graph unit, across (down is half, as cells are tall)
    pub scale: f64,
    /// the graph point in the middle
    pub cx: f64,
    pub cy: f64,
}

/// A terminal cell is about twice as tall as it is wide.
pub const ASPECT: f64 = 0.5;

impl Xform {
    pub fn to_screen(&self, x: f64, y: f64) -> (f64, f64) {
        (self.mid_x + (x - self.cx) * self.scale, self.mid_y + (y - self.cy) * self.scale * ASPECT)
    }

    pub fn to_graph(&self, sx: f64, sy: f64) -> (f64, f64) {
        (self.cx + (sx - self.mid_x) / self.scale, self.cy + (sy - self.mid_y) / (self.scale * ASPECT))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xform_round_trips() {
        let x = Xform { mid_x: 40.0, mid_y: 12.0, scale: 1.5, cx: 3.0, cy: -2.0 };
        let (sx, sy) = x.to_screen(10.0, 7.0);
        let (gx, gy) = x.to_graph(sx, sy);
        assert!((gx - 10.0).abs() < 1e-9 && (gy - 7.0).abs() < 1e-9);
    }
}
