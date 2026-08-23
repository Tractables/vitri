//! Graphviz (DOT) rendering of a vtree.
//!
//! [`vtree_to_dot`] draws the tree: leaves as boxes labelled with their 1-based
//! DIMACS variable, internal nodes as circles labelled with their node index —
//! the same numbering the `.vtree` file uses, so a circle in the picture and a
//! line in the file are the same node. It is a pure string function; rendering
//! is Graphviz's job:
//!
//! ```text
//! dot -Tsvg vtree.dot > vtree.svg
//! ```
//!
//! Handed a [`VtreeDotAnnotations`], each annotated node also gets a heatmap
//! fill and a short label beside it. That table is per-node `(heat, label)` and
//! nothing else — it names no CNF, no diagram and no compiler type — so a
//! consumer can colour the same tree by ITS own per-node measurements (nodes
//! materialised, apply cost, live width) instead of by ours.
//! [`annotate_from_cnf`] fills it with the two numbers this crate already
//! computes per node — clause load and context width — which are the quantities
//! [`VtreeScores`](crate::score::VtreeScores) summarises over the whole
//! tree.

use crate::cnf::CnfFormula;
use crate::score::{vtree_clause_load_per_node, vtree_context_width_per_node};
use crate::vtree::{Vtree, VtreeIdx};

/// Per-node annotations for [`vtree_to_dot`], indexed by [`VtreeIdx`].
///
/// A node may carry a `heat` in `[0,1]`, drawn as a fill colour on a light
/// yellow → orange → dark red scale, and a short `label`, drawn beside it. Both
/// are per node and independent: a node with neither is rendered plain, and a
/// node the set does not cover at all is rendered plain too.
///
/// Deliberately free of any formula or compiler type — it is the seam at which
/// an outside compiler feeds its OWN per-node statistics into the same picture.
/// [`annotate_from_cnf`] is one filler of it, not the only one.
#[derive(Clone, Debug, Default)]
pub struct VtreeDotAnnotations {
    nodes: Vec<NodeAnnotation>,
}

/// One node's slot. Both halves optional, so "no heat but a label" and "heat but
/// no label" are expressible without a sentinel value.
#[derive(Clone, Debug, Default)]
struct NodeAnnotation {
    label: Option<String>,
    heat: Option<f64>,
}

impl VtreeDotAnnotations {
    /// An annotation set covering `num_nodes` nodes, every one of them empty.
    /// Size it to the `vtree.num_nodes()` of the vtree it will be rendered with.
    pub fn new(num_nodes: usize) -> Self {
        VtreeDotAnnotations {
            nodes: vec![NodeAnnotation::default(); num_nodes],
        }
    }

    /// Colour `node` by `heat` in `[0,1]`. Out-of-range and NaN values are
    /// clamped/zeroed rather than rejected, so a caller normalising by a
    /// maximum it computed itself cannot produce an unrenderable colour.
    ///
    /// # Panics
    ///
    /// Panics if `node` is outside the `num_nodes` this set was built with.
    pub fn set_heat(&mut self, node: VtreeIdx, heat: f64) {
        let heat = if heat.is_nan() {
            0.0
        } else {
            heat.clamp(0.0, 1.0)
        };
        self.nodes[node.idx()].heat = Some(heat);
    }

    /// Label `node`. Keep it short — it is drawn at 8pt beside the node, and it
    /// is escaped for the DOT HTML-like string it lands in, so any text is safe.
    ///
    /// # Panics
    ///
    /// Panics if `node` is outside the `num_nodes` this set was built with.
    pub fn set_label(&mut self, node: VtreeIdx, label: impl Into<String>) {
        self.nodes[node.idx()].label = Some(label.into());
    }

    /// The clamped heat stored for `node`, or `None` when it was never set — an
    /// out-of-range `node` reads back `None` rather than panicking.
    pub fn heat(&self, node: VtreeIdx) -> Option<f64> {
        self.nodes.get(node.idx()).and_then(|n| n.heat)
    }

    /// The label stored for `node` unescaped, or `None` when it was never set — an
    /// out-of-range `node` reads back `None` rather than panicking.
    pub fn label(&self, node: VtreeIdx) -> Option<&str> {
        self.nodes.get(node.idx()).and_then(|n| n.label.as_deref())
    }
}

/// Render `vtree` as a Graphviz `graph`, optionally annotated.
///
/// Without `ann` this is the bare structure. With it, every annotated node gains
/// its heatmap fill and its label; nodes the set leaves empty are unchanged, so
/// a partial annotation is a legitimate input rather than a hole to fill.
///
/// Leaf labels are **1-based DIMACS** variables, matching the `.vtree` and
/// `.cnf` files this vtree is emitted beside; internal labels are node indices
/// in the same numbering as the `.vtree` file's own `L`/`I` lines.
pub fn vtree_to_dot(vtree: &Vtree, ann: Option<&VtreeDotAnnotations>) -> String {
    let mut dot = String::from("graph vtree {\n    rankdir=TB;\n");

    for (t, var) in vtree.leaf_bottomup() {
        dot.push_str(&format!(
            "    v{} [shape=box, label=\"X{}\"{}];\n",
            t.0,
            subscript(var.to_dimacs() as u32),
            decoration(ann, t),
        ));
    }
    for (t, _left, _right) in vtree.internal_bottomup() {
        dot.push_str(&format!(
            "    v{} [shape=circle, label=\"{}\"{}];\n",
            t.0,
            t.0,
            decoration(ann, t),
        ));
    }
    for (t, left, right) in vtree.internal_bottomup() {
        dot.push_str(&format!("    v{} -- v{};\n", t.0, left.0));
        dot.push_str(&format!("    v{} -- v{};\n", t.0, right.0));
    }

    dot.push_str("}\n");
    dot
}

/// Heat for every loaded node when the load is flat: low on the scale, but
/// distinct from the `0.0` of nodes carrying no clause at all.
const FLAT_LOAD_HEAT: f64 = 0.25;

/// The annotations this crate can derive for a vtree from the CNF it is for:
///
/// - **heat** — the node's *clause load* (the number of clauses whose variables
///   first meet there, i.e. whose literal leaves have it as their LCA) divided
///   by the largest clause load in the tree. Every node gets one, so the picture
///   shows where the formula piles up; a formula with no clauses at all leaves
///   the whole tree at `0.0`. When every loaded node carries the *same* load
///   there is no pile-up to show, so those nodes sit low on the scale instead
///   of all rendering as the maximum — a uniform tree reads calm, not alarming.
/// - **label** — `c=<clause load> w=<context width>`, on INTERNAL nodes only.
///   Leaves are left textless on purpose: a leaf's width is fixed by its single
///   variable, so labelling every one of them buries the internal nodes the
///   annotation is about.
///
/// `show_mask` is the projection show set over `formula`, or `None` for a plain
/// instance. It selects which context width is reported — over show variables under a projection, over all variables
/// otherwise — matching the `peak_context_width_show`/`peak_context_width_all` split in
/// [`VtreeScores`](crate::score::VtreeScores).
///
/// `formula` must be the CNF this vtree is FOR: a component's own CNF, in that
/// component's local numbering, for a component vtree.
pub fn annotate_from_cnf(
    vtree: &Vtree,
    formula: &CnfFormula,
    show_mask: Option<&crate::cnf::ShowMask>,
) -> VtreeDotAnnotations {
    let load = vtree_clause_load_per_node(vtree, formula);
    let width = vtree_context_width_per_node(vtree, formula, show_mask);
    let max_load = load.iter().copied().max().unwrap_or(0);
    let min_loaded = load.iter().copied().filter(|&c| c > 0).min();
    let flat = min_loaded == Some(max_load);

    let mut ann = VtreeDotAnnotations::new(vtree.num_nodes());
    for (i, &c) in load.iter().enumerate() {
        let heat = if max_load == 0 || c == 0 {
            0.0
        } else if flat {
            FLAT_LOAD_HEAT
        } else {
            c as f64 / max_load as f64
        };
        ann.set_heat(VtreeIdx(i as u32), heat);
    }
    for (t, _left, _right) in vtree.internal_bottomup() {
        ann.set_label(t, format!("c={} w={}", load[t.idx()], width[t.idx()]));
    }
    ann
}

/// The optional half of a node's attribute list: the heatmap fill, then the
/// label. Empty when the node carries no annotation.
fn decoration(ann: Option<&VtreeDotAnnotations>, node: VtreeIdx) -> String {
    let Some(ann) = ann else { return String::new() };
    let mut out = String::new();
    if let Some(heat) = ann.heat(node) {
        let (fill, font) = heatmap_color(heat);
        out.push_str(&format!(
            ", style=filled, fillcolor=\"{fill}\", fontcolor=\"{font}\""
        ));
    }
    if let Some(label) = ann.label(node) {
        // `xlabel`, not `label`: an EXTERNAL label is placed after layout, so
        // annotating a node never moves the tree. The shape a reader sees is the
        // vtree's, not the annotation's.
        out.push_str(&format!(
            ", xlabel=<<FONT COLOR=\"#888888\" POINT-SIZE=\"8\">{}</FONT>>",
            escape_html(label),
        ));
    }
    out
}

/// Map a normalized intensity `t ∈ [0,1]` to a fill colour and a font colour
/// that stays legible on it. Scale: light yellow (0.0) → orange (0.5) → dark
/// red (1.0). Returns `(fillcolor_hex, fontcolor_name)`.
fn heatmap_color(t: f64) -> (String, &'static str) {
    let (r, g, b) = if t < 0.5 {
        lerp_rgb((0xff, 0xff, 0xb2), (0xfd, 0x8d, 0x3c), t * 2.0)
    } else {
        lerp_rgb((0xfd, 0x8d, 0x3c), (0x80, 0x00, 0x26), (t - 0.5) * 2.0)
    };
    let fontcolor = if t > 0.65 { "white" } else { "black" };
    (format!("#{r:02x}{g:02x}{b:02x}"), fontcolor)
}

/// Componentwise linear interpolation between two RGB anchors at `t ∈ [0,1]`.
fn lerp_rgb(lo: (u8, u8, u8), hi: (u8, u8, u8), t: f64) -> (u8, u8, u8) {
    let lerp = |a: u8, b: u8| (a as f64 + t * (b as f64 - a as f64)).round() as u8;
    (lerp(lo.0, hi.0), lerp(lo.1, hi.1), lerp(lo.2, hi.2))
}

/// Escape a caller's label for the DOT HTML-like string it is placed in, where
/// these four characters are markup rather than text.
fn escape_html(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(c),
        }
    }
    out
}

/// A number in Unicode subscript digits, for the `Xₙ` leaf labels.
fn subscript(n: u32) -> String {
    const SUBSCRIPT_DIGITS: [char; 10] = ['₀', '₁', '₂', '₃', '₄', '₅', '₆', '₇', '₈', '₉'];
    if n == 0 {
        return SUBSCRIPT_DIGITS[0].to_string();
    }
    let mut digits = Vec::new();
    let mut rem = n;
    while rem > 0 {
        digits.push(SUBSCRIPT_DIGITS[(rem % 10) as usize]);
        rem /= 10;
    }
    digits.reverse();
    digits.into_iter().collect()
}

#[cfg(test)]
mod tests;
