//! CNF formula types.
//!
//! - `Literal`: a variable with a polarity
//! - `Clause`: a disjunction of literals
//! - `CnfFormula`: a conjunction of clauses
//!
//! plus the header metadata a competition instance declares (`Mode`,
//! `WeightTable`, `CnfMeta`). Reading those types out of DIMACS text, and
//! writing them back, is `dimacs`; splitting a formula at its independent
//! components is `components`.

mod components;
mod dimacs;
mod literal;
pub(crate) mod show_set;
pub(crate) mod space;
pub(crate) mod weights;

pub(crate) use literal::EquivFold;
/// A variable identifier and a literal over it — the two types every other CNF
/// type is built from.
pub use literal::{Literal, VarId};

/// The projection show set and the mask derived from it.
pub use show_set::{ShowMask, ShowSet};

/// The marker types that say which formula's variables a numbering is
/// expressed over.
pub use space::{Local, Original, Reduced, Space};

/// The literal weights a weighted instance declares, and the resolved
/// per-variable table every stage reads them through.
pub use weights::{WeightTable, Weights};

pub(crate) use dimacs::{DimacsHeader, parse_weight, rational_string, write_dimacs};

/// Parse a DIMACS weight token into an exact rational.
pub use dimacs::parse_rational_weight;

/// The independent components of a clause slice, for a caller holding clauses
/// it has not wrapped in a formula.
pub use components::detect_components_in;

/// A clause: a disjunction of literals.
///
/// Each variable must appear at most once. A variable carrying both polarities
/// makes the clause a tautology, which [`CnfFormula::from_dimacs`] drops on the
/// way in; a clause built directly is taken at its word, so a programmatic
/// caller owes the uniqueness itself.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Clause {
    /// The disjuncts. Order matches the source DIMACS line after
    /// normalization (sorted by variable, deduplicated) once parsed via
    /// [`CnfFormula::from_dimacs`]; callers constructing a `Clause` directly
    /// are not required to sort.
    pub literals: Vec<Literal>,
}

impl Clause {
    /// Wraps `literals` as a `Clause`. Debug builds assert the
    /// at-most-once-per-variable invariant documented on [`Clause`]; release
    /// builds trust the caller and skip the O(k²) check.
    pub fn new(literals: Vec<Literal>) -> Self {
        debug_assert!(
            {
                let mut ok = true;
                for i in 0..literals.len() {
                    for j in (i + 1)..literals.len() {
                        if literals[i].var == literals[j].var {
                            ok = false;
                            break;
                        }
                    }
                    if !ok {
                        break;
                    }
                }
                ok
            },
            "Clause::new: duplicate variable in literals {literals:?}",
        );
        Clause { literals }
    }
}

/// A `Clause` derefs to its literal slice, so `&Clause` coerces to `&[Literal]`
/// wherever a plain literal slice is expected — code that needs only the
/// literals takes no dependency on this type.
impl std::ops::Deref for Clause {
    type Target = [Literal];
    #[inline(always)]
    fn deref(&self) -> &[Literal] {
        &self.literals
    }
}

/// What preprocessing must preserve.
///
/// The first four are the MCC 2026 tracks and are the only tokens a `c t` line
/// may carry: `mc` (Track 1), `wmc` (weighted, Track 2 and the WMC sub-case of
/// Track 4), `pmc` (projected, Track 3), `pwmc` (projected weighted, Track 4).
/// Defaults to `Mc` when no `c t` line is present.
///
/// [`Compile`](Self::Compile) is a fifth mode that no header can declare — it is
/// reachable only by asking for it explicitly (`--mode compile`,
/// [`crate::config::RunConfig::mode`]).
///
/// `#[non_exhaustive]`: a caller matching on this must carry a `_` arm. A track
/// added to a later competition is a new variant here, and it should not break
/// a build that already handles the tracks it knows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum Mode {
    /// Track 1: plain unweighted, unprojected model counting.
    #[default]
    Mc,
    /// Track 2 (and the weighted half of Track 4): counts are weighted by
    /// per-literal weights from `c p weight` lines.
    Wmc,
    /// Track 3: counting is projected onto the `c p show` variable set.
    Pmc,
    /// Track 4: both projected (onto `c p show`) and weighted (via
    /// `c p weight`).
    Pwmc,
    /// Preserve the function, not just a count: the reduced formula plus the
    /// record reconstruct the original Boolean function over the original
    /// variables. Only stages with a recorded reconstruction run — forced-literal
    /// propagation, equivalent-literal substitution, free-variable removal — so
    /// preprocessing is weaker than any counting mode's.
    Compile,
}

impl Mode {
    /// Every mode, in the order a message or a `--help` line offers them.
    ///
    /// The vocabulary itself is [`Mode::token`]'s match, which the compiler
    /// keeps exhaustive; this fixes the ORDER and is what [`Mode::names`] and
    /// [`Mode::parse_mode`] read, so an offer, a rejection and a parse cannot
    /// disagree about which spellings exist.
    const ALL: &'static [Mode] = &[Mode::Mc, Mode::Wmc, Mode::Pmc, Mode::Pwmc, Mode::Compile];

    /// Every `--mode` token, in table order — for a shell over this crate that
    /// offers the vocabulary it will accept rather than keeping a copy.
    pub fn names() -> impl Iterator<Item = &'static str> {
        Mode::ALL.iter().map(|m| m.token())
    }

    /// Parses a `c t <type>` token: any [`Mode::names`] entry except
    /// `compile`, which `c t` cannot name — that header names a competition
    /// track, and [`Mode::Compile`] is not one.
    pub(crate) fn parse_track(s: &str) -> Option<Self> {
        Mode::parse_mode(s).filter(|m| *m != Mode::Compile)
    }
    /// Parses a `--mode` token: any [`Mode::names`] entry. The inverse of
    /// [`Mode::token`] by construction — it is that spelling looked up.
    pub fn parse_mode(s: &str) -> Option<Self> {
        Mode::ALL.iter().copied().find(|m| m.token() == s)
    }
    /// The token naming this mode, the exact inverse of
    /// [`Mode::parse_mode`].
    pub fn token(self) -> &'static str {
        match self {
            Mode::Mc => "mc",
            Mode::Wmc => "wmc",
            Mode::Pmc => "pmc",
            Mode::Pwmc => "pwmc",
            Mode::Compile => "compile",
        }
    }
    /// True for the two tracks whose count is weighted (`Wmc`, `Pwmc`).
    ///
    /// False for `Compile`, which carries any declared weights through untouched
    /// rather than counting under them — so a site asking "does this file declare
    /// weights" must read [`CnfMeta::declared_weights`], not this.
    pub fn is_weighted(self) -> bool {
        matches!(self, Mode::Wmc | Mode::Pwmc)
    }
    /// True for the two tracks whose count is projected onto a `show` variable
    /// set (`Pmc`, `Pwmc`). False for `Compile`, for the same reason as
    /// [`Self::is_weighted`].
    pub fn is_projected(self) -> bool {
        matches!(self, Mode::Pmc | Mode::Pwmc)
    }
}

/// CNF header metadata parsed from MCC `c t` / `c p show` / `c p weight`
/// meta-comment lines, returned alongside the [`CnfFormula`] by
/// [`CnfFormula::from_dimacs`]. All fields default to the Track-1
/// (plain MC) interpretation, so a file without these lines yields
/// `CnfMeta::default()`. The metadata is expressed over ORIGINAL DIMACS
/// variable ids and must be threaded and remapped explicitly by the caller as
/// preprocessing renumbers variables.
#[derive(Clone, Debug, Default)]
pub struct CnfMeta {
    /// The declared counting track; `Mode::Mc` if no `c t` line was seen.
    pub mode: Mode,
    /// The show set for projected counting, read through
    /// [`CnfMeta::declared_show_vars`]; the projected-out set is
    /// `all_vars \ show_vars`. `None` when no `c p show` line is present.
    show_vars: Option<ShowSet<Original>>,
    /// Literal weights for weighted counting, read through
    /// [`CnfMeta::declared_weights`]; `None` when no `c p weight` line is
    /// present.
    pub(crate) weights: Option<WeightTable>,
}

impl CnfMeta {
    /// The show set this file declares, or `None` when it carried no
    /// `c p show` line.
    ///
    /// Declaring a show set and counting under one are different questions: a
    /// projected mode (`pmc`/`pwmc`) uses this set as the projection its
    /// preprocessing preserves; [`Mode::Compile`] carries a declared set through
    /// without projecting, keying on this rather than [`Mode::is_projected`];
    /// an unprojected mode ignores it (see
    /// [`ResolvedMode::notices`](crate::config::ResolvedMode::notices)).
    ///
    /// An empty set is still a declaration: `c p show 0` projects onto
    /// nothing (count is 1 or 0), unlike an unprojected count over the same
    /// clauses.
    pub fn declared_show_vars(&self) -> Option<&ShowSet<Original>> {
        self.show_vars.as_ref()
    }

    /// The literal weights this file declares, or `None` when it carried no
    /// `c p weight` line.
    ///
    /// Declaring weights and counting under them are different questions, as
    /// they are for the show set above: a weighted mode (`wmc`/`pwmc`) counts
    /// under this table, [`Mode::Compile`] renumbers it onto the reduced
    /// formula and carries it through rather than folding it into the lift,
    /// and an unweighted mode ignores it (see
    /// [`ResolvedMode::notices`](crate::config::ResolvedMode::notices)).
    pub fn declared_weights(&self) -> Option<&WeightTable> {
        self.weights.as_ref()
    }
}

/// A CNF formula: a conjunction of clauses.
///
/// `PartialEq`/`Eq` are structural (same declared `num_vars`, same clauses in
/// the same order) — an identity check, not semantic equivalence.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CnfFormula {
    /// Declared variable count from the DIMACS `p cnf <vars> <clauses>`
    /// header, and the whole variable space: a file naming an id above it is
    /// rejected by [`CnfFormula::from_dimacs`], so this is never
    /// lower than the widest id in `clauses`.
    pub num_vars: u32,
    /// The conjuncts. May exceed the header's advisory clause count — extra
    /// clauses beyond the declared total are accepted, not truncated.
    pub clauses: Vec<Clause>,
}

impl CnfFormula {
    /// The refutation over `num_vars` variables: one empty clause, so nothing
    /// satisfies it, and the declared variable space intact, so a caller's
    /// numbering still reads over it.
    pub(crate) fn contradiction(num_vars: u32) -> Self {
        CnfFormula {
            num_vars,
            clauses: vec![Clause::new(vec![])],
        }
    }

    /// Whether this formula carries a refutation —
    /// [`contains_empty_clause`] over its own clauses.
    pub(crate) fn is_refuted(&self) -> bool {
        contains_empty_clause(&self.clauses)
    }
}

/// Whether `clauses` contains the empty clause — the form in which every pass
/// that derives a contradiction reports one, and the one spelling of the
/// question, so that a clause slice and a whole formula cannot answer it
/// differently.
pub(crate) fn contains_empty_clause(clauses: &[Clause]) -> bool {
    clauses.iter().any(|c| c.literals.is_empty())
}

/// Derived per-variable views of a clause set: occurrence lists, appearance
/// mask, frequency tables. Here rather than under a consumer because both the
/// preprocessing passes and vtree construction read them, and they are
/// functions of the clauses alone.
pub(crate) mod occ;

/// Disjoint-set helper, used here to group the variables a clause set connects
/// into independent components.
mod union_find;

/// Whole-formula shape statistics and the `coloring_like` predicate over them,
/// read by the Arjun bounded-variable-addition policy and by the vtree
/// portfolio's candidate gates.
pub(crate) mod stats;
