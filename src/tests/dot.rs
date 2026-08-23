use crate::cnf::CnfFormula;
use crate::cnf::{Reduced, ShowSet};
use crate::dot::*;
use crate::score::vtree_clause_load_per_node;
use crate::tests::dot_fixture::{fixture, node_line};
use crate::vtree::Vtree;
use crate::vtree::VtreeIdx;

#[test]
fn clause_loads_and_labels_follow_the_hand_computed_lcas() {
    let (vtree, formula) = fixture();

    // leaf 1 takes the unit clause, internal 4 the two clauses over {1,2},
    // internal 5 the one over {3,4}, the root the one that spans both.
    assert_eq!(
        vtree_clause_load_per_node(&vtree, &formula),
        vec![0, 1, 0, 0, 2, 1, 1]
    );

    let ann = annotate_from_cnf(&vtree, &formula, None);
    // Context width: variable 1 crosses internal 4 (its widest clause meets
    // at the root), variable 4 crosses internal 5, nothing crosses the root.
    assert_eq!(ann.label(VtreeIdx(4)), Some("c=2 w=1"));
    assert_eq!(ann.label(VtreeIdx(5)), Some("c=1 w=1"));
    assert_eq!(ann.label(VtreeIdx(6)), Some("c=1 w=0"));
    assert_eq!(
        ann.label(VtreeIdx(1)),
        None,
        "leaves take the colour, not the text"
    );
    assert_eq!(
        ann.heat(VtreeIdx(4)),
        Some(1.0),
        "the heaviest node is the top of the scale"
    );
    assert_eq!(ann.heat(VtreeIdx(0)), Some(0.0));

    let dot = vtree_to_dot(&vtree, Some(&ann));
    assert_eq!(
        node_line(&dot, 4),
        "    v4 [shape=circle, label=\"4\", style=filled, fillcolor=\"#800026\", \
         fontcolor=\"white\", xlabel=<<FONT COLOR=\"#888888\" POINT-SIZE=\"8\">c=2 w=1</FONT>>];",
    );
    // An unloaded leaf sits at the bottom of the same scale, and stays a box.
    assert!(node_line(&dot, 0).contains("shape=box"));
    assert!(node_line(&dot, 0).contains("fillcolor=\"#ffffb2\""));
}

/// A projected instance reports context width over show variables, which is
/// the width a projected compile actually carries.
#[test]
fn a_show_mask_switches_the_reported_width() {
    let (vtree, formula) = fixture();
    // Variable 1 (the one crossing internal 4) projected out; variable 4 kept.
    let mask = ShowSet::<Reduced>::from_zero_based([1, 2, 3]).mask(4);
    let ann = annotate_from_cnf(&vtree, &formula, Some(&mask));
    assert_eq!(
        ann.label(VtreeIdx(4)),
        Some("c=2 w=0"),
        "the crossing var is hidden"
    );
    assert_eq!(ann.label(VtreeIdx(5)), Some("c=1 w=1"), "this one is kept");
}

#[test]
fn an_unannotated_render_is_the_bare_structure() {
    let (vtree, _) = fixture();
    let dot = vtree_to_dot(&vtree, None);

    assert!(dot.starts_with("graph vtree {\n"), "{dot}");
    assert_eq!(dot.matches("shape=box").count(), 4, "one box per leaf");
    assert_eq!(
        dot.matches("shape=circle").count(),
        3,
        "one circle per internal node"
    );
    assert_eq!(
        dot.matches(" -- ").count(),
        6,
        "two undirected edges per internal node"
    );
    assert_eq!(
        dot.matches('{').count(),
        dot.matches('}').count(),
        "balanced braces"
    );
    assert!(!dot.contains("fillcolor"), "no annotations, no colour");
    assert!(!dot.contains("xlabel"), "no annotations, no labels");

    // Leaves carry the 1-based DIMACS variable, never the 0-based internal id.
    assert!(dot.contains("label=\"X₁\""), "{dot}");
    assert!(dot.contains("label=\"X₄\""), "{dot}");
    assert!(!dot.contains("X₀"), "there is no variable 0: {dot}");
}

/// The annotation table carries whatever a caller puts in it — the renderer
/// asks nothing about where the numbers came from.
#[test]
fn any_caller_can_annotate_any_node() {
    let (vtree, _) = fixture();
    let mut ann = VtreeDotAnnotations::new(vtree.num_nodes());
    ann.set_label(VtreeIdx(6), "peak <live> width");
    ann.set_heat(VtreeIdx(6), 4.0); // out of range, clamped to the top
    assert_eq!(ann.heat(VtreeIdx(6)), Some(1.0));

    let dot = vtree_to_dot(&vtree, Some(&ann));
    assert!(
        node_line(&dot, 6).contains("peak &lt;live&gt; width"),
        "labels are escaped: {dot}"
    );
    assert!(
        !node_line(&dot, 5).contains("xlabel"),
        "unannotated nodes stay plain"
    );
    assert!(!node_line(&dot, 5).contains("fillcolor"));
}

/// A formula with no clauses piles up nowhere, so the whole tree is at the
/// bottom of the scale — not at a flat maximum, and not a division by zero.
#[test]
fn a_formula_with_no_clauses_leaves_the_whole_tree_cold() {
    let vtree = Vtree::balanced(4);
    let formula = CnfFormula {
        num_vars: 4,
        clauses: Vec::new(),
    };
    let ann = annotate_from_cnf(&vtree, &formula, None);
    for node in 0..vtree.num_nodes() as u32 {
        assert_eq!(
            ann.heat(VtreeIdx(node)),
            Some(0.0),
            "node {node} carries no clause, so it carries no heat",
        );
    }
    assert_eq!(ann.label(VtreeIdx(6)), Some("c=0 w=0"));
    let dot = vtree_to_dot(&vtree, Some(&ann));
    assert!(
        !dot.contains("#800026"),
        "an empty formula must not render at the alarm end of the scale:\n{dot}",
    );
}

/// A caller normalising by a maximum it computed itself can hand over a value
/// that is not a number, or one below the scale. Both are stored as the bottom
/// of the scale rather than refused, so no annotation can produce an
/// unrenderable colour.
#[test]
fn a_heat_that_is_not_a_number_is_stored_as_zero_rather_than_rejected() {
    let mut ann = VtreeDotAnnotations::new(3);
    ann.set_heat(VtreeIdx(0), f64::NAN);
    assert_eq!(ann.heat(VtreeIdx(0)), Some(0.0));
    ann.set_heat(VtreeIdx(1), -2.0);
    assert_eq!(
        ann.heat(VtreeIdx(1)),
        Some(0.0),
        "below the scale is the bottom of it",
    );
}

/// Reading is total: a node the set does not cover, and a covered node whose
/// other half was never set, both read back as absent rather than panicking or
/// inventing a value.
#[test]
fn an_annotation_outside_the_set_reads_back_as_absent() {
    let mut ann = VtreeDotAnnotations::new(3);
    assert_eq!(ann.heat(VtreeIdx(99)), None);
    assert_eq!(ann.label(VtreeIdx(99)), None);

    ann.set_label(VtreeIdx(2), "only text");
    assert_eq!(ann.label(VtreeIdx(2)), Some("only text"));
    assert_eq!(
        ann.heat(VtreeIdx(2)),
        None,
        "the two halves of a slot are independent",
    );
}

/// A label lands in a DOT HTML-like string, where four characters are markup
/// rather than text. All four are escaped on the way out; the stored label is
/// what the caller wrote.
#[test]
fn every_markup_character_in_a_label_is_escaped() {
    let (vtree, _) = fixture();
    let written = r#"a & b < c > d "e""#;
    let mut ann = VtreeDotAnnotations::new(vtree.num_nodes());
    ann.set_label(VtreeIdx(6), written);
    assert_eq!(
        ann.label(VtreeIdx(6)),
        Some(written),
        "the stored label is unescaped",
    );
    let dot = vtree_to_dot(&vtree, Some(&ann));
    assert!(
        node_line(&dot, 6).contains("a &amp; b &lt; c &gt; d &quot;e&quot;"),
        "all four markup characters must be escaped: {dot}",
    );
}

/// Leaf labels are subscripted decimal, so a variable past nine needs every one
/// of its digits rather than the last.
#[test]
fn leaf_labels_carry_subscript_digits_past_nine() {
    let dot = vtree_to_dot(&Vtree::balanced(12), None);
    for label in ["X₉", "X₁₀", "X₁₂"] {
        assert!(
            dot.contains(&format!("label=\"{label}\"")),
            "{label}: {dot}"
        );
    }
    assert!(
        !dot.contains("X1"),
        "the digits are subscript throughout, never plain: {dot}",
    );
}
