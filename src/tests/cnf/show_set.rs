//! The show set's own invariants: what the constructors canonicalize, where
//! the 1-based DIMACS form begins and ends, and how a set descends into a
//! component.

use crate::cnf::ShowSet;
use crate::cnf::VarId;
use crate::cnf::{Local, Original, Reduced};

/// Whatever order and repeats a producer hands over, a set comes out ascending
/// and deduplicated — every reader downstream, the `c p show` writer included,
/// depends on that without checking it.
#[test]
fn a_set_is_ascending_and_deduplicated_however_it_was_built() {
    let from_ids = ShowSet::<Original>::from_dimacs_ids(&[4, 1, 4, 2]).expect("1-based ids");
    assert_eq!(from_ids.to_dimacs(), vec![1, 2, 4]);
    assert_eq!(
        from_ids.iter_vars().collect::<Vec<_>>(),
        vec![VarId(0), VarId(1), VarId(3)],
    );

    let from_vars = ShowSet::<Original>::from_zero_based([3, 0, 3, 1]);
    assert_eq!(from_vars.as_zero_based(), &[0, 1, 3]);
    assert_eq!(from_vars, from_ids);
}

/// `0` closes a `c p show` line; it never names a variable. Reading one as an
/// id would silently produce the last variable of the space instead.
#[test]
fn zero_is_not_a_show_variable() {
    let err = ShowSet::<Reduced>::from_dimacs_ids(&[1, 0, 2]).expect_err("0 must be refused");
    assert!(
        err.to_string().contains("0 is not a show variable"),
        "{err}"
    );
}

/// An empty declaration is a set, not the absence of one.
#[test]
fn the_empty_set_shows_nothing_and_writes_nothing() {
    let empty = ShowSet::<Original>::empty();
    assert!(empty.is_empty());
    assert_eq!(empty.len(), 0);
    assert!(empty.to_dimacs().is_empty());
    assert_eq!(empty, ShowSet::from_dimacs_ids(&[]).unwrap());
}

/// Masking is against a NAMED formula, and a set may legitimately be wider than
/// the one being masked (a component's, say) — the ids it does not have are
/// dropped rather than rejected, so the mask is the intersection.
#[test]
fn the_mask_drops_ids_the_masked_formula_does_not_have() {
    let set = ShowSet::<Reduced>::from_dimacs_ids(&[1, 3]).unwrap();
    assert_eq!(set.mask(4).as_slice(), &[true, false, true, false]);
    assert_eq!(set.mask(2).as_slice(), &[true, false]);
    assert_eq!(set.mask(0).as_slice(), &[] as &[bool]);
    assert_eq!(set.mask(4).count(), 2);
    assert!(set.mask(4).is_show(VarId(2)));
    assert!(!set.mask(4).is_show(VarId(9)));
}

/// The defined-variable fold appends to a set that is already canonical, so the
/// insert has to place its variable rather than push it.
#[test]
fn insert_keeps_the_set_canonical_and_is_idempotent() {
    let mut set = ShowSet::<Reduced>::from_zero_based([1, 5]);
    set.insert(VarId(3));
    set.insert(VarId(0));
    set.insert(VarId(5));
    assert_eq!(set.as_zero_based(), &[0, 1, 3, 5]);
    assert_eq!(set.len(), 4);
}

/// Restricting to a component renumbers into its dense local space: local `i`
/// is shown iff the variable it stands for is. Ascending by construction, since
/// the walk is in local order.
#[test]
fn restrict_renumbers_a_set_into_a_components_own_space() {
    let global = ShowSet::<Reduced>::from_dimacs_ids(&[1, 8, 11]).unwrap();
    let mask = global.mask(11);
    let component = [VarId(6), VarId(7), VarId(8), VarId(9), VarId(10)];
    let local: ShowSet<Local> = mask.restrict(&component);
    assert_eq!(local.to_dimacs(), vec![2, 5]);
    assert!(mask.restrict(&[VarId(1), VarId(2)]).is_empty());
}

/// The one place an ORIGINAL set may be read as a REDUCED one without a map,
/// and it says so in its name.
#[test]
fn assuming_the_identity_keeps_every_variable() {
    let original = ShowSet::<Original>::from_dimacs_ids(&[2, 5]).unwrap();
    let reduced: ShowSet<Reduced> = original.clone().assume_reduced_identity();
    assert_eq!(reduced.to_dimacs(), original.to_dimacs());
}

/// The serde module writes the same 1-based ascending array the `c p show` line
/// beside it carries, and reads it back to the same set.
#[test]
fn the_serde_module_writes_the_one_based_array() {
    #[derive(serde::Serialize, serde::Deserialize, PartialEq, Debug)]
    struct Holder {
        #[serde(default, with = "crate::cnf::show_set::dimacs")]
        show: Option<ShowSet<Reduced>>,
    }

    let held = Holder {
        show: Some(ShowSet::from_zero_based([2, 0])),
    };
    let json = serde_json::to_string(&held).unwrap();
    assert_eq!(json, r#"{"show":[1,3]}"#);
    assert_eq!(serde_json::from_str::<Holder>(&json).unwrap(), held);

    let absent = Holder { show: None };
    let json = serde_json::to_string(&absent).unwrap();
    assert_eq!(json, r#"{"show":null}"#);
    assert_eq!(serde_json::from_str::<Holder>(&json).unwrap(), absent);

    assert!(serde_json::from_str::<Holder>(r#"{"show":[0]}"#).is_err());
}
