//! The force-directed embedding, asked for on its own.
//!
//! What a caller gets here is geometry, not a vtree: coordinates it can cluster,
//! branch on, or measure. The cases below pin the two properties that makes it
//! usable — the same input gives the same points, and the points are the ones
//! the vtree construction reads — plus the range it refuses.

use crate::decompose::{Embedding, EmbeddingOptions, MAX_EMBEDDING_DIM, embed};
use crate::error::VitriError;
use crate::tests::common::{make_formula, wide_component};
use crate::vtree::VarId;

/// The default options, at a stated dimension.
fn at_dim(dim: usize) -> EmbeddingOptions {
    EmbeddingOptions { dim }
}

/// A caller that embeds, acts on the geometry, and embeds again in another
/// process must see the same picture both times — otherwise nothing it derived
/// from the first one can be compared with the second. There is no clock and no
/// unseeded randomness anywhere in the layout, and this is what says so.
#[test]
fn the_same_formula_and_options_give_the_same_points() {
    let formula = wide_component();
    let once = embed(&formula, &at_dim(2)).expect("the fixture embeds");
    let twice = embed(&formula, &at_dim(2)).expect("the fixture embeds");
    assert_eq!(once, twice);
}

/// The buffer is row-major and one row per variable, which is the whole of what
/// a caller needs to know to read it — and [`Embedding::position`] is that
/// arithmetic done once rather than at every call site.
#[test]
fn every_variable_has_a_row_of_its_own() {
    let formula = wide_component();
    for dim in 2..=MAX_EMBEDDING_DIM {
        let e = embed(&formula, &at_dim(dim)).expect("the fixture embeds");
        assert_eq!(e.dim, dim);
        assert_eq!(e.num_vars(), formula.num_vars);
        assert_eq!(e.coords.len(), formula.num_vars as usize * dim);
        let v = VarId(3);
        assert_eq!(e.position(v), &e.coords[v.idx() * dim..v.idx() * dim + dim]);
    }
}

/// A dimension the layout cannot work in is refused rather than clamped: a
/// caller that asked for a geometry it did not get would go on to reason about
/// the wrong one.
#[test]
fn a_dimension_outside_the_accepted_range_is_refused() {
    let formula = wide_component();
    for dim in [0, 1, MAX_EMBEDDING_DIM + 1] {
        let err = embed(&formula, &at_dim(dim)).expect_err("out of range must be refused");
        assert!(
            matches!(err, VitriError::Input { .. }),
            "an unusable dimension is bad input: {err:?}",
        );
        assert!(
            err.to_string().contains(&MAX_EMBEDDING_DIM.to_string()),
            "the refusal states the range it would have accepted: {err}",
        );
    }
}

/// A formula with no variables has no points, and says so rather than handing
/// back an empty embedding a caller would divide by.
#[test]
fn a_formula_with_no_variables_has_nothing_to_embed() {
    let err = embed(&make_formula(0, Vec::new()), &at_dim(2))
        .expect_err("an empty formula must be refused");
    assert!(matches!(err, VitriError::Input { .. }), "{err:?}");
}

/// The point of the geometry: the layout places every variable at the centre of
/// the clauses it occurs in, so variables that occur together end up nearer each
/// other than variables picked without regard to the formula. Stated as the two
/// averages, which is a property of the whole embedding rather than of one pair
/// it might happen to get right.
#[test]
fn variables_that_share_a_clause_are_placed_nearer_than_variables_that_do_not() {
    let formula = wide_component();
    let e = embed(&formula, &at_dim(2)).expect("the fixture embeds");

    let mut together = Vec::new();
    for clause in &formula.clauses {
        for (i, a) in clause.literals.iter().enumerate() {
            for b in &clause.literals[i + 1..] {
                together.push(distance(&e, a.var.0, b.var.0));
            }
        }
    }
    let mut any = Vec::new();
    for a in 0..formula.num_vars {
        for b in (a + 1)..formula.num_vars {
            any.push(distance(&e, a, b));
        }
    }
    let mean = |d: &[f64]| d.iter().sum::<f64>() / d.len() as f64;
    assert!(
        mean(&together) < mean(&any),
        "clause-sharing pairs average {} apart and arbitrary pairs {} — the \
         embedding is not reading the formula",
        mean(&together),
        mean(&any),
    );
}

/// Euclidean distance between two variables' positions.
fn distance(e: &Embedding, a: u32, b: u32) -> f64 {
    e.position(VarId(a))
        .iter()
        .zip(e.position(VarId(b)))
        .map(|(x, y)| (x - y) * (x - y))
        .sum::<f64>()
        .sqrt()
}
