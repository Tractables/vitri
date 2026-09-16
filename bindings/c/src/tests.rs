//! What a C program cannot reach on purpose: a panic stopped at the boundary,
//! and the status code of each error kind. `tests/test_vitri.c` covers the
//! rest through the header.

use std::collections::BTreeSet;

use super::*;

#[test]
fn a_panic_comes_back_as_the_panic_code_with_its_message() {
    let result = answer(|| panic!("the invariant broke"));
    assert_eq!(result.code, VITRI_ERROR_PANIC);
    let failure = result.failure().expect("a panic is a failed result");
    assert_eq!(failure.kind.bytes(), b"panic");
    let message = String::from_utf8_lossy(failure.message.bytes());
    assert!(
        message.contains("the invariant broke"),
        "the message should carry the panic's: {message}"
    );
}

#[test]
fn each_error_kind_has_a_code_of_its_own() {
    let errors = [
        VitriError::config("reason"),
        VitriError::spec("spec", "reason"),
        VitriError::env("VITRI_VARIABLE", "reason"),
        VitriError::input("reason"),
        VitriError::mismatch("reason"),
        VitriError::construction("spec", "reason"),
        VitriError::io("file", "read", &std::io::Error::other("reason")),
    ];
    let codes: BTreeSet<vitri_code> = errors.iter().map(code_of).collect();
    assert_eq!(
        codes.len(),
        errors.len(),
        "two kinds share a code: {codes:?}"
    );
    for reserved in [
        VITRI_OK,
        VITRI_ERROR_OTHER,
        VITRI_ERROR_INVALID_ARGUMENT,
        VITRI_ERROR_PANIC,
    ] {
        assert!(
            !codes.contains(&reserved),
            "a vitri error kind was given the reserved code {reserved}"
        );
    }
}
