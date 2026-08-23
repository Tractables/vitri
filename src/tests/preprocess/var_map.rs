use crate::cnf::{Reduced, ShowSet, VarId, Weights, parse_weight};
use crate::preprocess::simplify::OriginalFate;
use crate::preprocess::{OriginalMap, OriginalTarget, VarMap};

/// The pair of spaces these tests read the algebra in. Which pair it is does
/// not matter to any of them — one that inversion maps onto itself just keeps
/// the expected values in the same type as the map under test.
type Map = VarMap<Reduced, Reduced>;

#[test]
fn identity_names_every_variable_as_itself() {
    assert_eq!(
        Map::identity(3),
        Map::from_entries(vec![Some(1), Some(2), Some(3)])
    );
    assert!(Map::identity(0).is_empty());
}

#[test]
fn injectivity_rejects_aliasing_and_out_of_range_entries() {
    let m = Map::from_entries(vec![Some(2), None, Some(-1)]);
    assert!(m.is_injective(2));
    // Two source variables naming target variable 1, polarity notwithstanding.
    assert!(!Map::from_entries(vec![Some(1), Some(-1)]).is_injective(2));
    // An entry naming a target variable the formula does not have.
    assert!(!m.is_injective(1));
}

#[test]
fn inversion_preserves_polarity_and_leaves_introduced_variables_unnamed() {
    // Source var 0 IS target var 2; source var 2 is the NEGATION of target var
    // 1; target var 3 was introduced by preprocessing.
    let m = Map::from_entries(vec![Some(2), None, Some(-1)]);
    assert_eq!(
        m.invert(3),
        Map::from_entries(vec![Some(-3), Some(1), None])
    );
}

#[test]
fn inversion_composes_an_earlier_stages_naming() {
    let m = Map::from_entries(vec![Some(2), Some(-1)]);
    // The earlier stage named source variable `j` as original variable `j + 10`.
    let composed = m.invert_composed(2, |source_var| source_var as i32 + 10);
    assert_eq!(composed, Map::from_entries(vec![Some(-11), Some(10)]));
    // Inverting twice returns to the original correspondence.
    assert_eq!(m.invert(2).invert(2), m);
}

#[test]
fn a_map_serializes_as_the_bare_array_of_entries() {
    let m = Map::from_entries(vec![Some(2), None, Some(-1)]);
    assert_eq!(serde_json::to_string(&m).unwrap(), "[2,null,-1]");
}

/// The carry runs in SOURCE-variable order, so its output is ascending without
/// a sort, and a source variable this map leaves unnamed (one preprocessing
/// INTRODUCED) is never a show variable: there is no target variable for it to
/// inherit the declaration from.
#[test]
fn carry_show_is_ascending_and_drops_introduced_variables() {
    // SOURCE 0 stands for target 2 (show), 1 was introduced, 2 for target 0
    // (show), 3 for target 1 (not show).
    let map = Map::from_entries(vec![Some(3), None, Some(1), Some(2)]);
    let target_show = ShowSet::<Reduced>::from_zero_based([0, 2]);
    assert_eq!(map.carry_show(&target_show).to_dimacs(), vec![1, 3],);

    let introduced = Map::from_entries(vec![None, None]);
    assert!(introduced.carry_show(&target_show).is_empty());
    assert!(
        Map::from_entries(vec![Some(2)])
            .carry_show(&target_show)
            .is_empty()
    );
}

/// A source variable named by a NEGATIVE entry stands for the target variable's
/// NEGATION, so its positive literal carries the target's negative weight and
/// the pair comes back swapped. Getting that backwards leaves every count right
/// and every lifted model's weight wrong, which is why the swap is spelled in
/// one place and asserted here rather than at a caller.
#[test]
fn carry_weights_swaps_the_pair_of_a_negated_entry() {
    let w = |s: &str| parse_weight(s).expect("an exact rational");
    // SOURCE 0 stands for target 1, source 1 for the NEGATION of target 0,
    // source 2 was introduced by preprocessing.
    let map = Map::from_entries(vec![Some(2), Some(-1), None]);
    let target = Weights::<Reduced>::from_dimacs_pairs(
        &[
            (1, w("1/3")),
            (-1, w("2/5")),
            (2, w("4/7")),
            (-2, w("6/11")),
        ],
        2,
    );
    assert_eq!(
        map.carry_weights(&target).as_pairs(),
        [
            (w("6/11"), w("4/7")),
            (w("1/3"), w("2/5")),
            (w("1/1"), w("1/1")),
        ],
    );
}

/// An entry is a signed 1-based literal, so `0` names nothing — a map carrying
/// one is not injective into any space and no stage may be accepted on it.
#[test]
fn injectivity_rejects_an_entry_naming_no_variable() {
    assert!(!Map::from_entries(vec![Some(1), Some(0)]).is_injective(2));
}

/// Past the validator, the readers of a stored entry drop the one they cannot
/// name rather than computing on it: inversion leaves its target unnamed, and
/// the two carries leave the source variable without a declaration or a weight
/// of its own.
#[test]
fn the_readers_of_an_entry_naming_no_variable_drop_it() {
    let map = Map::from_entries(vec![Some(1), Some(0)]);
    assert_eq!(
        map.invert_composed(2, |source_var| source_var as i32 + 10),
        Map::from_entries(vec![Some(10), None]),
    );

    let target_show = ShowSet::<Reduced>::from_zero_based([0, 1]);
    assert_eq!(map.carry_show(&target_show).to_dimacs(), vec![1],);

    let w = |s: &str| parse_weight(s).expect("an exact rational");
    let target = Weights::<Reduced>::from_dimacs_pairs(&[(1, w("1/3")), (-1, w("2/5"))], 2);
    assert_eq!(
        map.carry_weights(&target).as_pairs(),
        [(w("2/5"), w("1/3")), (w("1/1"), w("1/1"))],
    );
}

/// The one crossing from what the simplification reported into the on-disk
/// numbering: each of the three fates has its own entry kind, a 0-based reduced
/// index becomes a 1-based DIMACS id, and an original equal to the NEGATION of
/// its reduced variable is the sign on that id.
#[test]
fn each_original_fate_becomes_its_own_map_entry_kind() {
    let map = OriginalMap::from_fates(&[
        OriginalFate::Variable {
            index: 2,
            same_polarity: true,
        },
        OriginalFate::Variable {
            index: 0,
            same_polarity: false,
        },
        OriginalFate::Forced(true),
        OriginalFate::Forced(false),
        OriginalFate::Unconstrained,
    ]);

    assert_eq!(
        map,
        OriginalMap::from_entries(vec![
            OriginalTarget::Literal(3),
            OriginalTarget::Literal(-1),
            OriginalTarget::Constant(true),
            OriginalTarget::Constant(false),
            OriginalTarget::Free,
        ]),
    );
}

/// A run that removed nothing still owes a total map, and it is the one that
/// renames nothing: original variable `i` is reduced variable `i`, in the same
/// 1-based numbering every other entry uses.
#[test]
fn the_identity_original_map_names_every_variable_as_its_own_reduced_one() {
    let map = OriginalMap::identity(3);

    assert_eq!(map.len(), 3, "one entry per original variable");
    assert_eq!(map.get(VarId(0)), Some(OriginalTarget::Literal(1)));
    assert_eq!(map.get(VarId(2)), Some(OriginalTarget::Literal(3)));
    assert_eq!(
        map.get(VarId(3)),
        None,
        "an id the original formula never had has no entry",
    );
    assert!(OriginalMap::identity(0).is_empty());
}
