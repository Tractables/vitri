//! The `Display` string is what a consumer prints, so it is pinned here: each
//! variant must name its own subject (the spec, the variable, the backend, the
//! path) without the caller having to add anything.

use crate::error::VitriError;

#[test]
fn display_names_the_offending_spec() {
    let e = VitriError::spec("flowcutter-primal:bogus", "invalid parameter 'bogus'");
    assert_eq!(
        e.to_string(),
        "vtree spec 'flowcutter-primal:bogus': invalid parameter 'bogus'",
    );
}

#[test]
fn display_names_the_environment_variable() {
    let e = VitriError::env("VITRI_ARJUN_SBVA", "must be on, off or auto; got \"yes\"");
    assert_eq!(
        e.to_string(),
        "environment variable VITRI_ARJUN_SBVA: must be on, off or auto; got \"yes\"",
    );
}

#[test]
fn display_names_the_backend() {
    let e = VitriError::construction("minfill", "empty formula");
    assert_eq!(e.to_string(), "minfill failed: empty formula");
}

#[test]
fn display_names_the_file_and_what_was_attempted() {
    let e = VitriError::Io {
        path: "/tmp/x.vtree".into(),
        action: "read",
        reason: "No such file or directory (os error 2)".to_string(),
    };
    assert_eq!(
        e.to_string(),
        "cannot read /tmp/x.vtree: No such file or directory (os error 2)",
    );
}

/// Config and input failures already state their whole case, so nothing is
/// prefixed onto them.
#[test]
fn display_passes_config_and_input_through_unchanged() {
    assert_eq!(
        VitriError::config("candidates must be at least 1").to_string(),
        "candidates must be at least 1",
    );
    assert_eq!(
        VitriError::input("the vtree has 3 leaves but the formula has 4 variables").to_string(),
        "the vtree has 3 leaves but the formula has 4 variables",
    );
}

/// The crate's own binary decides the exit code by matching on the variant, so
/// the type has to be usable as a `std::error::Error` by anyone else too.
#[test]
fn is_a_standard_error() {
    fn takes_error(_: &dyn std::error::Error) {}
    takes_error(&VitriError::config("x"));
}

/// A mismatch says which two arguments disagree and how, so nothing is prefixed
/// onto it either — unlike the four variants that name a subject first.
#[test]
fn a_mismatch_error_prints_its_reason_unchanged() {
    let reason = "component 0 vtree has 4 leaves but its CNF has 5 variables";
    assert_eq!(VitriError::mismatch(reason).to_string(), reason);
}

/// Every variant states its whole case in its own sentence, so a caller that
/// prints the message alone loses nothing — there is no second error underneath
/// to walk to. The io variant is the one that could have carried one and
/// deliberately does not: its source is flattened into the message.
#[test]
fn no_variant_carries_a_source_because_each_states_its_whole_case() {
    use std::error::Error as _;
    let cause = std::io::Error::new(std::io::ErrorKind::NotFound, "No such file or directory");
    let every_variant = [
        VitriError::config("a request that contradicts itself"),
        VitriError::spec("minfill:best=on", "the family cannot rank"),
        VitriError::env("VITRI_TEST", "must be a count"),
        VitriError::input("the file declares no variables"),
        VitriError::mismatch("the two arguments are about different formulas"),
        VitriError::construction("minfill", "out of time"),
        VitriError::io("/x.vtree", "read", &cause),
    ];
    assert_eq!(
        every_variant.len(),
        7,
        "a variant added without a case here would go unchecked",
    );
    for e in &every_variant {
        assert!(
            e.source().is_none(),
            "{e} points at a second error the message does not carry",
        );
    }
}

/// The io constructor is where a failed file operation becomes a message: it
/// keeps the verb that was attempted and the path, and flattens the operating
/// system's account into the sentence.
#[test]
fn an_io_failure_names_the_verb_that_was_attempted() {
    let cause = std::io::Error::new(
        std::io::ErrorKind::PermissionDenied,
        "Permission denied (os error 13)",
    );
    let e = VitriError::io("/nowhere/vtree.vtree", "write", &cause);
    assert_eq!(
        e.to_string(),
        "cannot write /nowhere/vtree.vtree: Permission denied (os error 13)",
    );
    match &e {
        VitriError::Io { path, action, .. } => {
            assert_eq!(*action, "write", "the verb is kept as its own field");
            assert_eq!(path.as_os_str(), "/nowhere/vtree.vtree");
        }
        other => panic!("the constructor must build an Io error, got {other:?}"),
    }
}
