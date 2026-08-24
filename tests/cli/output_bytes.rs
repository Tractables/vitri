//! The literal bytes of the output files

use super::*;

/// The self-description lines are matched as LITERAL TEXT, not by parsing them
/// back: a writer bug mirrored by an equal-and-opposite reader bug round-trips
/// perfectly and still ships a file no other consumer can read. Single spaces,
/// the trailing `0`, and the exact `c t <track>` spelling are the contract.
#[test]
fn the_self_description_lines_are_the_exact_documented_text() {
    let t = Scratch::new("headertext");

    // One fixture per track, each declaring exactly what its mode needs.
    let cases: [(&str, &str); 4] = [
        ("mc", "p cnf 2 1\n1 2 0\n"),
        (
            "wmc",
            "c t wmc\np cnf 2 1\nc p weight 1 1/3 0\nc p weight -1 2/3 0\n1 2 0\n",
        ),
        ("pmc", "c t pmc\np cnf 2 1\nc p show 1 2 0\n1 2 0\n"),
        (
            "pwmc",
            "c t pwmc\np cnf 2 1\nc p show 1 2 0\nc p weight 1 1/3 0\nc p weight -1 2/3 0\n1 2 0\n",
        ),
    ];
    for (track, text) in cases {
        let input = t.file(&format!("{track}.cnf"), text);
        let out = t.out(track);
        // `--no-arjun` alone for the two projected tracks: they have no simplify
        // chain, so `--no-simplify` is refused there. Every stage the mode HAS
        // is off either way, which is all this fixture needs.
        let mut args = vec![s(&input), "-o", s(&out), "--no-arjun"];
        if !track.starts_with('p') {
            args.push("--no-simplify");
        }
        run(&args).exit(0);
        let cnf = read(&out.join(REDUCED_CNF_NAME));

        assert!(
            cnf.lines().any(|l| l == format!("c t {track}")),
            "expected the exact line `c t {track}` in:\n{cnf}",
        );
        if track.starts_with('p') {
            assert!(
                cnf.lines().any(|l| l == "c p show 1 2 0"),
                "the show line is single-spaced and `0`-terminated:\n{cnf}",
            );
        }
        if track.contains('w') {
            for want in ["c p weight 1 1/3 0", "c p weight -1 2/3 0"] {
                assert!(
                    cnf.lines().any(|l| l == want),
                    "expected the exact line `{want}` in:\n{cnf}",
                );
            }
        }
    }
}

/// `components.json` states the on-disk contract it was written under, as the
/// literal tag `docs/bundle.md` publishes — not merely as whatever constant
/// the writer happened to hold, which would let the constant itself drift away
/// from the document without a test noticing.
#[test]
fn the_components_manifest_carries_its_published_format_tag() {
    let t = Scratch::new("compstag");
    let input = t.file("in.cnf", IRREDUCIBLE_5);
    let out = t.out("bundle");
    run(&[s(&input), "-o", s(&out)]).exit(0);

    let manifest = json(&out.join(COMPONENTS_JSON_NAME));
    assert_eq!(manifest["format"], "vitri-components-v1");
    assert_eq!(manifest["format"], COMPONENTS_FORMAT_TAG);
}

/// The manifest's field set, exactly — the one assertion that catches a
/// renamed, added or dropped key before a consumer does. The two candidate set fields
/// are ABSENT by default rather than present-and-empty, which is what keeps a
/// default bundle unchanged for a reader that predates the candidate set.
#[test]
fn the_components_manifest_has_exactly_the_documented_keys() {
    let t = Scratch::new("compskeys");
    let input = t.file("in.cnf", TWO_COMPONENTS);
    let out = t.out("bundle");
    run(&[s(&input), "-o", s(&out)]).exit(0);

    let manifest = json(&out.join(COMPONENTS_JSON_NAME));
    assert_eq!(
        keys(&manifest),
        set(&["format", "free_vars_reduced_dimacs", "components"]),
    );
    let components = manifest["components"].as_array().expect("components[]");
    assert!(!components.is_empty(), "the array is never empty");
    for entry in components {
        assert_eq!(
            keys(entry),
            set(&[
                "local_to_reduced_dimacs",
                "show_vars_local_dimacs",
                "selection",
                "cnf",
                "vtree",
            ]),
        );
        assert_eq!(
            keys(&entry["selection"]),
            set(&["winning_spec", "tree_decomposition"]),
        );
        assert_eq!(
            keys(&entry["selection"]["tree_decomposition"]),
            set(&["num_bags", "treewidth"]),
        );
    }
}

/// Every component says which construction produced its vtree, and a TD-based
/// one publishes the shape of the decomposition behind it — the summary, never
/// the per-variable bag map.
///
/// The fixture's components are small enough to skip the portfolio, which is
/// the case where the manifest is the ONLY record of what built them: a
/// single-candidate component retains no candidate set to read it off.
#[test]
fn every_component_names_the_construction_that_produced_its_vtree() {
    let t = Scratch::new("selection");
    let input = t.file("in.cnf", TWO_COMPONENTS);
    let out = t.out("bundle");
    run(&[s(&input), "-o", s(&out)]).exit(0);

    let manifest = json(&out.join(COMPONENTS_JSON_NAME));
    let components = manifest["components"].as_array().expect("components[]");
    assert_eq!(components.len(), 2, "the fixture is disconnected");
    for entry in components {
        let selection = &entry["selection"];
        assert_eq!(
            selection["winning_spec"], "minfill-primal",
            "a component under the portfolio threshold is built by min-fill",
        );
        let td = &selection["tree_decomposition"];
        let vars = entry["local_to_reduced_dimacs"]
            .as_array()
            .expect("the local map")
            .len() as u64;
        let num_bags = td["num_bags"].as_u64().expect("num_bags");
        let treewidth = td["treewidth"].as_u64().expect("treewidth");
        assert!(
            num_bags >= 1 && num_bags <= vars,
            "a decomposition of {vars} variables has 1..={vars} bags, got {num_bags}",
        );
        assert!(
            treewidth < vars,
            "the largest bag holds at most all {vars} of the component's own \
             variables, so the width stays below {vars}, got {treewidth}",
        );
    }
}

/// A construction that decomposes nothing still names itself, and omits the
/// tree-decomposition block rather than writing an empty or zeroed one.
#[test]
fn a_construction_without_a_decomposition_names_itself_and_omits_the_block() {
    let t = Scratch::new("selectionforce");
    let input = t.file("in.cnf", TWO_COMPONENTS);
    let out = t.out("bundle");
    // `whole` keeps the small components off the min-fill path, so the spec
    // asked for is the spec that runs.
    run(&[
        s(&input),
        "-o",
        s(&out),
        "--vtree",
        "force",
        "--components",
        "whole",
    ])
    .exit(0);

    let manifest = json(&out.join(COMPONENTS_JSON_NAME));
    let entry = &manifest["components"][0];
    assert_eq!(entry["selection"]["winning_spec"], "force");
    assert_eq!(
        keys(&entry["selection"]),
        set(&["winning_spec"]),
        "the force-directed embedding decomposes nothing, so there is no block to write",
    );
}

/// `reduced.cnf` states the whole problem on its own: a consumer that never
/// opens `preprocess.json` still counts the right thing. Re-derived here by
/// parsing ONLY the CNF and comparing with the record it was written beside.
#[test]
fn reduced_cnf_alone_states_the_problem_it_belongs_to() {
    let t = Scratch::new("selfdesc");
    let input = t.file("pw.cnf", PROJECTED_WEIGHTED);
    let out = t.out("bundle");
    run(&[s(&input), "-o", s(&out)]).exit(0);

    let record = json(&out.join(PREPROCESS_RECORD_NAME));
    let file = std::fs::File::open(out.join(REDUCED_CNF_NAME)).expect("open reduced.cnf");
    let (_, meta) =
        CnfFormula::from_dimacs(std::io::BufReader::new(file)).expect("the emitted CNF must parse");

    assert_eq!(meta.mode.token(), record["mode"]);

    let show_from_cnf: Vec<u64> = meta
        .declared_show_vars()
        .expect("a projected file declares its show set")
        .to_dimacs()
        .into_iter()
        .map(u64::from)
        .collect();
    let show_from_record: Vec<u64> = record["show_vars_reduced_dimacs"]
        .as_array()
        .expect("the record carries the same set")
        .iter()
        .map(|v| v.as_u64().expect("a variable id"))
        .collect();
    assert_eq!(show_from_cnf, show_from_record);

    // The two files weigh every literal identically, in the same order — both
    // are written from the same table, so anything else means one of the two
    // emitters re-derived the numbering.
    let weights = meta
        .declared_weights()
        .expect("a weighted file declares its weights");
    let num_vars = record["reduced_to_original_dimacs"]
        .as_array()
        .expect("the map")
        .len();
    let from_cnf: Vec<(i64, String)> = weights
        .resolve::<Reduced>(num_vars)
        .to_record_rows()
        .into_iter()
        .map(|row| (i64::from(row.literal), row.weight))
        .collect();
    let from_record: Vec<(i64, String)> = record["reduced_weights"]
        .as_array()
        .expect("weights")
        .iter()
        .map(|w| {
            (
                w["literal"].as_i64().expect("a literal"),
                w["weight"].as_str().expect("a weight string").to_string(),
            )
        })
        .collect();
    assert_eq!(
        from_cnf, from_record,
        "`reduced.cnf` and `preprocess.json` must weigh the same literals the same way",
    );
}

/// Every rational this crate writes is canonical: lowest terms, positive
/// denominator. Preprocessing is free to fold weights together, so the output
/// values are not the input's — what has to hold is the FORM, on every one of
/// them, including the scalar lift.
///
/// `compile` is the mode that carries a weight table through untouched apart
/// from renumbering, so it is the one that puts the input's own awkward
/// spellings (`2/4`, `-1/-3`, `6/-8`) in front of the writer.
#[test]
fn every_written_rational_is_in_lowest_terms_with_a_positive_denominator() {
    fn assert_canonical(what: &str, weight: &str) {
        let (num, den) = weight
            .split_once('/')
            .unwrap_or_else(|| panic!("{what} must be written as num/den, got {weight:?}"));
        let num: i64 = num
            .parse()
            .unwrap_or_else(|_| panic!("{what} numerator {num:?}"));
        let den: i64 = den
            .parse()
            .unwrap_or_else(|_| panic!("{what} denominator {den:?}"));
        assert!(
            den > 0,
            "{what} must carry the sign on the numerator: {weight}"
        );
        let (mut a, mut b) = (num.unsigned_abs(), den.unsigned_abs());
        while b != 0 {
            let t = a % b;
            a = b;
            b = t;
        }
        assert_eq!(a, 1, "{what} is not in lowest terms: {weight}");
    }

    let t = Scratch::new("rationals");

    let input = t.file("pw.cnf", PROJECTED_WEIGHTED);
    let out = t.out("compile");
    run(&[s(&input), "-o", s(&out), "--mode", "compile"]).exit(0);
    let record = json(&out.join(PREPROCESS_RECORD_NAME));
    assert_canonical("weight_lift", record["weight_lift"].as_str().expect("lift"));
    let written: Vec<String> = record["reduced_weights"]
        .as_array()
        .expect("weights")
        .iter()
        .map(|w| w["weight"].as_str().expect("a weight").to_string())
        .collect();
    for w in &written {
        assert_canonical("a reduced weight", w);
    }
    assert!(
        written.iter().any(|w| w == "1/2"),
        "`2/4` must be written back reduced, got {written:?}",
    );
    assert!(
        written.iter().any(|w| w == "-3/4"),
        "`6/-8` must move the sign onto the numerator, got {written:?}",
    );

    // ...and the lift itself, when preprocessing actually produces one: a free
    // variable under a weighted mode contributes `w⁻ + w⁺`, which is 2/3 here.
    let free = t.file(
        "free.cnf",
        "c t wmc\np cnf 2 1\nc p weight 2 1/3 0\nc p weight -2 1/3 0\n1 0\n",
    );
    let out = t.out("lift");
    let r = run(&[s(&free), "-o", s(&out)]).exit(0);
    let lift = json(&out.join(PREPROCESS_RECORD_NAME))["weight_lift"]
        .as_str()
        .expect("lift")
        .to_string();
    assert_canonical("weight_lift", &lift);
    assert_eq!(lift, "2/3");
    // The printed factor is the weighted half alone: a power of two would be
    // meaningless under a weighted mode, and printing an inert `2^0` is noise.
    r.assert_stdout("count(original) = count(reduced) * 2/3");
}

/// The manifest records the spec the parser accepted, parameters included — so
/// feeding `winning_spec` back to `--vtree` rebuilds the construction that ran,
/// not the family default of the same base.
#[test]
fn the_manifest_records_the_parameters_the_winning_spec_was_built_with() {
    let t = Scratch::new("keyed-selection");
    // Wide enough to be built by the spec the caller named: a tiny component
    // takes the min-fill path whatever `--vtree` says, and reports THAT.
    let input = t.file("in.cnf", &wide_component_dimacs(None));
    let out = t.out("bundle");
    let spec = "minfill-incidence:seed=7";
    run(&[s(&input), "-o", s(&out), "--vtree", spec]).exit(0);

    let manifest = json(&out.join(COMPONENTS_JSON_NAME));
    for entry in manifest["components"].as_array().expect("components[]") {
        assert_eq!(
            entry["selection"]["winning_spec"], spec,
            "the manifest must name the spec that built this vtree, parameters included",
        );
    }
}
