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

/// A programmatic declaration table owns only the rows its caller supplied,
/// and assigning one literal twice has the same last-line-wins meaning as
/// repeated `c p weight` lines.
#[test]
fn a_programmatic_weight_table_is_sparse_and_the_last_duplicate_wins() {
    let w = |s: &str| parse_weight(s).expect("an exact rational");
    let table =
        WeightTable::from_dimacs_pairs(vec![(1, w("1/3")), (-3, w("2/5")), (1, w("7/9"))], 4)
            .expect("every literal is in the declared variable space");

    assert_eq!(
        table.to_literal_pairs(),
        vec![(1, w("7/9")), (-3, w("2/5"))],
        "an omitted literal must stay omitted rather than becoming an explicit weight 1",
    );
}

/// Absence and an explicit empty declaration remain different values at the
/// programmatic boundary, just as no `c p show` line differs from
/// `c p show 0` in a file. The track is the same kind of value: `None` is an
/// undeclared track, not `mc`.
#[test]
fn programmatic_metadata_preserves_empty_declarations_and_absence() {
    let empty_weights = WeightTable::from_dimacs_pairs(Vec::new(), 3)
        .expect("an empty table has no out-of-range literal");
    let declared = CnfMeta::from_parts(
        3,
        Some(Mode::Pwmc),
        Some(ShowSet::empty()),
        Some(empty_weights),
    )
    .expect("empty declarations are valid");
    assert_eq!(declared.declared_track(), Some(Mode::Pwmc));
    assert_eq!(declared.declared_show_vars(), Some(&ShowSet::empty()));
    assert_eq!(
        declared
            .declared_weights()
            .expect("the empty table is still declared")
            .to_literal_pairs(),
        Vec::new(),
    );

    let absent = CnfMeta::from_parts(3, None, None, None)
        .expect("absent declarations carry no ids to validate");
    assert_eq!(absent.declared_track(), None);
    assert_eq!(
        absent.mode(),
        Mode::Mc,
        "an undeclared track still resolves to plain model counting",
    );
    assert!(absent.declared_show_vars().is_none());
    assert!(absent.declared_weights().is_none());
}

/// A signed DIMACS weight id always passes through the declared formula range
/// gate: zero names no variable, and either polarity of an id above the count
/// is invalid caller input.
#[test]
fn zero_and_out_of_range_programmatic_weight_ids_are_input_errors() {
    let w = |s: &str| parse_weight(s).expect("an exact rational");
    for lit in [0, 4, -4, i32::MIN] {
        let err = WeightTable::from_dimacs_pairs(vec![(lit, w("1/2"))], 3)
            .expect_err("the literal is not in 1..=num_vars");
        assert!(
            matches!(err, crate::error::VitriError::Input { .. }),
            "malformed metadata is input, got {err:?}",
        );
        assert!(
            err.to_string().contains(&lit.to_string()),
            "{err} must name the offending literal {lit}",
        );
    }
}

/// Written show ids reject zero where the typed set is built, and its metadata
/// owner rejects a 0-based id that the formula does not declare. Together they
/// are the range gate for a programmatically assembled typed show set.
#[test]
fn zero_and_out_of_range_programmatic_show_ids_are_input_errors() {
    let zero = ShowSet::<Original>::from_dimacs_ids(&[0])
        .expect_err("zero terminates a written show line and names no variable");
    assert!(
        matches!(zero, crate::error::VitriError::Input { .. }),
        "malformed metadata is input, got {zero:?}",
    );
    assert!(zero.to_string().contains('0'), "{zero} must name zero");

    let err = CnfMeta::from_parts(
        3,
        Some(Mode::Pmc),
        Some(ShowSet::from_zero_based([3])),
        None,
    )
    .expect_err("zero-based id 3 is DIMACS variable 4, above num_vars 3");
    assert!(
        matches!(err, crate::error::VitriError::Input { .. }),
        "malformed metadata is input, got {err:?}",
    );
    assert!(err.to_string().contains('4'), "{err} must name DIMACS id 4");
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
