use crate::preprocess::cadical_ffi::*;

struct Collect(Vec<Vec<i32>>);
impl ClauseIterator for Collect {
    fn clause(&mut self, c: &[i32]) -> bool {
        self.0.push(c.to_vec());
        true
    }
}

struct Never;
impl Terminator for Never {
    fn terminated(&mut self) -> bool {
        false
    }
}

#[test]
fn solves_a_trivial_sat_instance() {
    let mut s = CaDiCal::new().expect("the solver allocates");
    for lit in [1, 2, 0, -1, 0] {
        s.add(lit);
    }
    assert_eq!(s.solve(), Status::Satisfiable);
    assert_eq!(s.val(1), -1);
    assert_eq!(s.val(2), 2);
}

#[test]
fn detects_unsatisfiable() {
    let mut s = CaDiCal::new().expect("the solver allocates");
    for lit in [1, 0, -1, 0] {
        s.add(lit);
    }
    assert_eq!(s.solve(), Status::Unsatisfiable);
}

#[test]
fn fixed_reports_units_and_traverse_sees_clauses() {
    let mut s = CaDiCal::new().expect("the solver allocates");
    for lit in [1, 0, -1, 2, 0] {
        s.add(lit);
    }
    s.freeze(1);
    s.freeze(2);
    Bounded::new(&mut s, Never).simplify(3);
    assert_eq!(s.fixed(1), 1, "unit clause (1) should be forced true");

    let mut c = Collect(Vec::new());
    s.traverse_clauses(&mut c);
    assert!(
        !c.0.is_empty(),
        "traversal should yield the irredundant clauses"
    );
}

#[test]
fn terminator_stops_a_pass() {
    struct Always;
    impl Terminator for Always {
        fn terminated(&mut self) -> bool {
            true
        }
    }
    let mut s = CaDiCal::new().expect("the solver allocates");
    for lit in [1, 2, 0, -1, 3, 0] {
        s.add(lit);
    }
    // Must return rather than hang; the status itself is not what is under
    // test (CaDiCaL may still finish such a tiny formula).
    let _ = Bounded::new(&mut s, Always).simplify(3);
}

/// Regression: traversing a formula CaDiCaL has refuted must not abort.
///
/// An empty clause reaches `clause_trampoline` as `len == 0` with a
/// possibly-null pointer (`std::vector::data()` may return null when the
/// vector is empty). Feeding that to `slice::from_raw_parts` is UB, and in a
/// debug build the precondition check makes it a NON-UNWINDING abort — the
/// whole test binary dies with SIGABRT rather than failing one test.
#[test]
fn traverse_survives_the_empty_clause() {
    let mut s = CaDiCal::new().expect("the solver allocates");
    s.add(1);
    s.add(0);
    s.add(-1);
    s.add(0);
    assert_eq!(s.solve(), Status::Unsatisfiable);

    // The assertion that matters is that this returns at all.
    let mut c = Collect(Vec::new());
    s.traverse_clauses(&mut c);
    assert!(
        c.0.iter().all(|cl| cl.len() <= 2),
        "refuted formula should yield only its own small clauses, got {:?}",
        c.0,
    );
}

#[test]
fn limit_accepts_a_known_name_and_rejects_junk() {
    let mut s = CaDiCal::new().expect("the solver allocates");
    assert!(s.limit(c"conflicts", 1000));
    assert!(!s.limit(c"definitely-not-a-cadical-limit", 1));
}
