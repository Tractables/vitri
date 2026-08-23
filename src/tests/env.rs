//! The `VITRI_*` reading rule, pinned on the pure halves.

use crate::env::*;
use crate::error::VitriError;

#[test]
fn unset_is_the_default_and_a_good_value_wins() {
    assert_eq!(parse_value("VITRI_TEST", None, 7u32, "a count").unwrap(), 7);
    assert_eq!(
        parse_value("VITRI_TEST", Some("50000"), 7u32, "a count").unwrap(),
        50_000
    );
    assert_eq!(
        parse_value("VITRI_TEST", Some("-3"), 0i64, "a signed count").unwrap(),
        -3
    );
}

/// The one rule for how a value is written, on the two readers that apply it:
/// a shell exports whitespace around a value easily and none of these values is
/// whitespace, and a value that is a word is a word whatever its case. Widening
/// only — every spelling accepted without the rule is still accepted with it.
#[test]
fn surrounding_whitespace_and_the_case_of_a_word_are_not_part_of_a_value() {
    assert!(is_form(" ON ", "on"));
    assert!(is_form("all", "all"));
    assert!(!is_form("all the way", "all"));

    assert_eq!(
        parse_value("VITRI_TEST", Some(" 2500 "), 0u64, "a count").unwrap(),
        2_500
    );
    for on in [" 1 ", "ON", "True", "\ton\n"] {
        assert!(flag_value("VITRI_TEST", Some(on), false).unwrap(), "{on}");
    }
    for off in [" 0 ", "OFF", "False"] {
        assert!(!flag_value("VITRI_TEST", Some(off), true).unwrap(), "{off}");
    }
}

/// The enumerated reader, which is the flag reader's own vocabulary generalized:
/// unset takes the default, a form is matched by the rule above, and anything
/// else is the one rejection shape naming the variable, what it accepts, and
/// the value as it was actually set.
#[test]
fn an_enumerated_value_off_the_vocabulary_is_an_error_naming_the_variable() {
    const FORMS: &[(&str, u8)] = &[("near", 1), ("far", 2)];
    assert_eq!(
        from_forms("VITRI_TEST", None, 9u8, FORMS, "near or far").unwrap(),
        9
    );
    assert_eq!(
        from_forms("VITRI_TEST", Some(" FAR "), 9u8, FORMS, "near or far").unwrap(),
        2
    );
    assert_eq!(
        from_forms("VITRI_TEST", Some(" middling "), 9u8, FORMS, "near or far"),
        Err(VitriError::env(
            "VITRI_TEST",
            "must be near or far; got \" middling \"".to_string()
        ))
    );
}

/// The empty string counts as set, not as unset.
#[test]
fn a_malformed_value_is_an_error_naming_the_variable() {
    for bad in ["not-a-number", "", " ", "12x"] {
        let err = parse_value("VITRI_TEST", Some(bad), 7u32, "a variable count")
            .expect_err("a value that does not parse must be reported");
        assert_eq!(
            err,
            VitriError::env(
                "VITRI_TEST",
                format!("must be a variable count; got {bad:?}")
            )
        );
        let shown = err.to_string();
        assert!(shown.contains("VITRI_TEST"), "{shown}");
        assert!(shown.contains("a variable count"), "{shown}");
    }
}

/// The on/off spellings, and the fact that anything else is an error rather
/// than a silent off.
#[test]
fn flag_spellings() {
    assert!(!flag_value("VITRI_TEST", None, false).unwrap());
    for on in ["1", "on", "true"] {
        assert!(flag_value("VITRI_TEST", Some(on), false).unwrap(), "{on}");
    }
    for off in ["0", "off", "false"] {
        assert!(
            !flag_value("VITRI_TEST", Some(off), false).unwrap(),
            "{off}"
        );
    }
    for bad in ["", "yes", "2"] {
        let err = flag_value("VITRI_TEST", Some(bad), false)
            .expect_err("a value that is neither on nor off must be reported");
        let shown = err.to_string();
        assert!(shown.contains("VITRI_TEST"), "{shown}");
        assert!(shown.contains("1, on or true"), "{shown}");
    }
}

/// A knob that ships ON reads the same spellings, but unset means on and the
/// error message says so — otherwise it would tell the reader the opposite of
/// what the code does.
#[test]
fn a_flag_that_ships_on_reports_unset_as_on() {
    assert!(flag_value("VITRI_TEST", None, true).unwrap());
    assert!(!flag_value("VITRI_TEST", Some("0"), true).unwrap());
    assert!(flag_value("VITRI_TEST", Some("1"), true).unwrap());
    let shown = flag_value("VITRI_TEST", Some("maybe"), true)
        .expect_err("a value that is neither on nor off must be reported")
        .to_string();
    assert!(shown.contains("unset is on"), "{shown}");
}
