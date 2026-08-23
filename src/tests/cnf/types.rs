use super::*;

#[test]
fn test_literal_negation() {
    let l = Literal::pos(VarId(0));
    let nl = l.negated();
    assert_eq!(nl.var, VarId(0));
    assert!(!nl.positive);
    assert_eq!(nl.negated(), l);
}

/// The entry for a DIMACS integer that has NOT been validated yet: `0`
/// terminates a clause rather than naming a variable, and `i32::MIN` names one
/// no DIMACS integer can write back, so neither reaches the arithmetic.
#[test]
fn try_from_dimacs_answers_none_for_what_names_no_variable() {
    assert_eq!(VarId::try_from_dimacs(1), Some(VarId(0)));
    assert_eq!(VarId::try_from_dimacs(-42), Some(VarId(41)));
    assert_eq!(VarId::try_from_dimacs(0), None);
    assert_eq!(VarId::try_from_dimacs(i32::MIN), None);
}

/// A written table is input: a pair naming no variable is dropped like one
/// naming a variable the table does not have, rather than being computed on.
#[test]
fn weights_from_pairs_drops_a_pair_naming_no_variable() {
    let w = |s: &str| parse_weight(s).expect("an exact rational");
    let table = Weights::<Original>::from_dimacs_pairs(&[(0, w("1/2")), (1, w("1/3"))], 1);
    assert_eq!(table.as_pairs(), [(w("1/1"), w("1/3"))]);
}

/// One vocabulary read three ways — the list a shell offers, the token a mode
/// writes, and the spelling a parse looks up — so a token has to survive the
/// trip through all three, in the order the list fixes.
#[test]
fn every_mode_token_parses_back_to_the_mode_that_wrote_it() {
    let names: Vec<&str> = Mode::names().collect();
    assert_eq!(names, ["mc", "wmc", "pmc", "pwmc", "compile"]);
    for name in names {
        let mode = Mode::parse_mode(name)
            .unwrap_or_else(|| panic!("{name} is offered but the parse does not accept it"));
        assert_eq!(
            mode.token(),
            name,
            "{name} parsed as a mode that writes itself differently",
        );
    }
}

/// A `c t` line names a competition track, and the compile mode is not one:
/// it is reachable only by asking for it, so a file cannot declare it.
#[test]
fn a_track_header_cannot_name_the_compile_mode() {
    assert_eq!(Mode::parse_track("compile"), None);
    for name in Mode::names().filter(|n| *n != "compile") {
        assert_eq!(
            Mode::parse_track(name),
            Mode::parse_mode(name),
            "{name} is a track and must parse as one",
        );
    }
    let err = CnfFormula::from_dimacs(std::io::Cursor::new("c t compile\np cnf 2 1\n1 2 0\n"))
        .expect_err("`c t compile` names no track");
    assert!(
        err.to_string().contains("compile"),
        "{err} must quote the token the line wrote",
    );
}

/// A refutation is a formula like any other: it keeps the variable space its
/// caller's numbering reads over, and says it has no models through the one
/// empty clause rather than by having no clauses.
#[test]
fn a_contradiction_keeps_its_variable_space_and_reports_itself_refuted() {
    let refuted = CnfFormula::contradiction(5);
    assert_eq!(
        refuted.num_vars, 5,
        "the declared variable space must survive the refutation",
    );
    assert_eq!(refuted.clauses, vec![Clause::new(Vec::new())]);
    assert!(refuted.is_refuted());

    let satisfiable = CnfFormula {
        num_vars: 5,
        clauses: vec![Clause::new(vec![Literal::pos(VarId(0))])],
    };
    assert!(
        !satisfiable.is_refuted(),
        "a formula with no empty clause is not a refutation",
    );
}

/// `e ≡ surv` multiplies the eliminated member's weights straight into its
/// survivor, and `e ≡ ¬surv` swaps them first — getting that backwards leaves
/// every count right and every lifted model's weight wrong.
#[test]
fn folding_an_anti_equivalent_partner_swaps_its_two_weights() {
    let w = |s: &str| parse_weight(s).expect("an exact rational");
    // The survivor weighs (w⁻, w⁺) = (3, 2); the partner folded in weighs (5, 7).
    let table = || Weights::<Original>::from_dimacs_pairs(&[(1, w("2")), (-1, w("3"))], 1);

    let mut same = table();
    same.fold_into((w("5"), w("7")), Literal::pos(VarId(0)));
    assert_eq!(
        same.as_pairs(),
        [(w("15"), w("14"))],
        "each polarity takes the partner's own: (3·5, 2·7)",
    );

    let mut opposite = table();
    opposite.fold_into((w("5"), w("7")), Literal::neg(VarId(0)));
    assert_eq!(
        opposite.as_pairs(),
        [(w("21"), w("10"))],
        "the negated fold takes the other polarity's: (3·7, 2·5)",
    );
}

/// A batch of folds is one fold per partner into its OWN survivor — the
/// polarity of each is read off that fold, not shared across the batch.
#[test]
fn folding_a_batch_multiplies_each_partner_into_its_own_survivor() {
    let w = |s: &str| parse_weight(s).expect("an exact rational");
    let mut weights = Weights::<Original>::from_dimacs_pairs(
        &[
            (1, w("2")),
            (-1, w("3")),
            (2, w("5")),
            (-2, w("7")),
            (3, w("11")),
            (-3, w("13")),
            (4, w("17")),
            (-4, w("19")),
        ],
        4,
    );

    weights.fold_eliminated(&[
        EquivFold {
            eliminated: VarId(1),
            survivor: Literal::pos(VarId(0)),
        },
        EquivFold {
            eliminated: VarId(3),
            survivor: Literal::neg(VarId(2)),
        },
    ]);

    assert_eq!(
        weights.as_pairs(),
        [
            // var 0 absorbs var 1 straight through: (3·7, 2·5).
            (w("21"), w("10")),
            (w("7"), w("5")),
            // var 2 absorbs var 3 with the polarities swapped: (13·17, 11·19).
            (w("221"), w("209")),
            (w("19"), w("17")),
        ],
    );
}

/// Eliminating a variable whose two literals weigh differently is non-scalar,
/// so this is the set a caller freezes out of the eliminating stages. A variable
/// weighing the same both ways stays eliminable even when that weight is not 1.
#[test]
fn unequal_vars_names_exactly_the_variables_whose_two_literals_differ() {
    let w = |s: &str| parse_weight(s).expect("an exact rational");
    // var 0 weighs 1/2 both ways, var 1 was never named so it weighs 1 both
    // ways, var 2's two literals differ.
    let weights = Weights::<Original>::from_dimacs_pairs(
        &[(1, w("1/2")), (-1, w("1/2")), (3, w("2/3")), (-3, w("1/3"))],
        3,
    );
    assert_eq!(
        weights.unequal_vars(),
        [VarId(2)].into_iter().collect(),
        "only a variable whose polarities weigh differently is frozen out",
    );
}
