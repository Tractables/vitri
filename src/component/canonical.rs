//! Canonical form of a component-local CNF, the identity under which the
//! per-build vtree cache recognises a repeated component.
//!
//! A component is numbered by where its variables sit in the outer formula, so
//! two copies of one gadget generally carry different local numberings. The
//! canonical form renames the variables by an order derived from the component's
//! own structure, so that copies related by a variable renaming (and any clause
//! reordering) come out as the same [`ComponentKey`].
//!
//! The order comes from colour refinement (1-dimensional Weisfeiler–Leman) on
//! the signed variable–clause incidence graph, followed by individualisation of
//! the variables refinement cannot tell apart. Colour ids are assigned by
//! sorting signatures, never by first appearance, so they do not depend on the
//! local numbering. When tied variables are related by an automorphism any
//! choice among them gives the same form; when they are not, two copies may be
//! given different forms and simply do not share an entry.
//!
//! Soundness does not rest on the refinement: a [`ComponentKey`] holds the
//! relabelled clause set itself, so equal keys are an exact isomorphism whatever
//! order was chosen. The refinement only decides how often copies meet.

use crate::cnf::{CnfFormula, ShowMask};
use crate::vtree::VarId;

/// Past this many variables plus clauses, ties left by refinement are broken by
/// local index instead of individualised. Individualisation refines once per
/// tied variable, so its cost grows with the square of the component; beyond
/// this size a repeated component is rare enough, and its build slow enough,
/// that a missed hit is cheaper than the search for one.
const MAX_INDIVIDUALISED_SIZE: usize = 2048;

/// Past this many variables plus clauses, a component is not refined at all and
/// keeps its local numbering, so its key is its local clause set as it stands.
/// Refinement takes one pass over the clauses per round and as many rounds as
/// the incidence graph needs to stop splitting colour classes, which on a
/// long, sparse component is a number that grows with its size. Copies of a
/// component this large are rare, and missing one costs nothing beyond the
/// build the old key would have done anyway.
const MAX_REFINED_SIZE: usize = 20_000;

/// Identity of a component up to a renaming of its variables: equal keys mean
/// the clause sets, and the show masks, are the same after relabelling.
#[derive(PartialEq, Eq, Hash, Debug)]
pub(super) struct ComponentKey {
    num_vars: u32,
    /// Each clause's `(canonical var, positive)` literals sorted, then the clause
    /// list sorted.
    clauses: Vec<Vec<(u32, bool)>>,
    /// The show mask the build would install, indexed by canonical variable
    /// (`None` where the build ignores the mask).
    show: Option<Vec<bool>>,
}

/// A component's [`ComponentKey`] and the renaming that produced it.
pub(super) struct CanonicalForm {
    pub(super) key: ComponentKey,
    /// Local variable index to canonical id.
    to_canonical: Vec<u32>,
}

impl CanonicalForm {
    /// The canonical id of a local variable.
    pub(super) fn to_canonical(&self, local: VarId) -> VarId {
        VarId::from_idx(self.to_canonical[local.idx()] as usize)
    }

    /// For each canonical id, the local variable it stands for.
    pub(super) fn canonical_to_local(&self) -> Vec<VarId> {
        let mut inverse = vec![VarId::from_idx(0); self.to_canonical.len()];
        for (local, &canon) in self.to_canonical.iter().enumerate() {
            inverse[canon as usize] = VarId::from_idx(local);
        }
        inverse
    }
}

/// The canonical form of `sub` under `show` (the local show mask the build
/// would see, or `None` when the build ignores it).
pub(super) fn canonical_form(sub: &CnfFormula, show: Option<&ShowMask>) -> CanonicalForm {
    let n = sub.num_vars() as usize;
    let clauses: Vec<Vec<(usize, bool)>> = sub
        .clauses()
        .iter()
        .map(|c| {
            c.literals
                .iter()
                .map(|l| (l.var.idx(), l.positive))
                .collect()
        })
        .collect();
    let order = refined_order(n, &clauses, show);
    let mut to_canonical = vec![0u32; n];
    for (rank, &local) in order.iter().enumerate() {
        to_canonical[local] = rank as u32;
    }

    let mut relabelled: Vec<Vec<(u32, bool)>> = clauses
        .iter()
        .map(|c| {
            let mut lits: Vec<(u32, bool)> = c.iter().map(|&(v, p)| (to_canonical[v], p)).collect();
            lits.sort_unstable();
            lits
        })
        .collect();
    relabelled.sort_unstable();
    let show = show.map(|m| {
        let mut by_canonical = vec![false; n];
        for (local, &canon) in to_canonical.iter().enumerate() {
            by_canonical[canon as usize] = m.is_show(VarId::from_idx(local));
        }
        by_canonical
    });
    CanonicalForm {
        key: ComponentKey {
            num_vars: sub.num_vars(),
            clauses: relabelled,
            show,
        },
        to_canonical,
    }
}

/// Variables in canonical order: sorted by final colour, ties (left only by a
/// component too large to individualise) by local index. A component too large
/// to refine ([`MAX_REFINED_SIZE`]) keeps its local order.
fn refined_order(n: usize, clauses: &[Vec<(usize, bool)>], show: Option<&ShowMask>) -> Vec<usize> {
    if n + clauses.len() > MAX_REFINED_SIZE {
        return (0..n).collect();
    }
    let mut occurrences: Vec<Vec<(usize, bool)>> = vec![Vec::new(); n];
    for (ci, c) in clauses.iter().enumerate() {
        for &(v, p) in c {
            occurrences[v].push((ci, p));
        }
    }
    let mut var_col: Vec<u32> = (0..n)
        .map(|v| u32::from(show.is_some_and(|m| m.is_show(VarId::from_idx(v)))))
        .collect();
    let mut clause_col: Vec<u32> = clauses.iter().map(|c| c.len() as u32).collect();
    refine(&occurrences, clauses, &mut var_col, &mut clause_col);

    if n + clauses.len() <= MAX_INDIVIDUALISED_SIZE {
        while let Some(pick) = first_tied_variable(&var_col) {
            // Doubling keeps the order of the classes and puts the picked
            // variable ahead of the rest of its own.
            for (v, c) in var_col.iter_mut().enumerate() {
                *c = *c * 2 + u32::from(v != pick);
            }
            for c in clause_col.iter_mut() {
                *c *= 2;
            }
            refine(&occurrences, clauses, &mut var_col, &mut clause_col);
        }
    }
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_unstable_by_key(|&v| (var_col[v], v));
    order
}

/// The lowest-indexed variable of the lowest-coloured class with more than one
/// member, or `None` when every class is a singleton.
fn first_tied_variable(var_col: &[u32]) -> Option<usize> {
    let mut by_colour: Vec<(u32, usize)> = var_col.iter().copied().zip(0..).collect();
    by_colour.sort_unstable();
    by_colour
        .windows(2)
        .find(|w| w[0].0 == w[1].0)
        .map(|w| w[0].1)
}

type Signature = (u32, Vec<(bool, u32)>);

/// Refine both colourings until the number of classes stops growing. Each round
/// a node's signature is its old colour plus the sorted multiset of
/// `(polarity, neighbour colour)`; new ids are the signatures' ranks.
fn refine(
    occurrences: &[Vec<(usize, bool)>],
    clauses: &[Vec<(usize, bool)>],
    var_col: &mut [u32],
    clause_col: &mut [u32],
) {
    let mut classes = class_count(var_col) + class_count(clause_col);
    loop {
        let var_sig: Vec<Signature> = occurrences
            .iter()
            .enumerate()
            .map(|(v, occ)| {
                let mut s: Vec<(bool, u32)> =
                    occ.iter().map(|&(ci, p)| (p, clause_col[ci])).collect();
                s.sort_unstable();
                (var_col[v], s)
            })
            .collect();
        let clause_sig: Vec<Signature> = clauses
            .iter()
            .enumerate()
            .map(|(ci, c)| {
                let mut s: Vec<(bool, u32)> = c.iter().map(|&(v, p)| (p, var_col[v])).collect();
                s.sort_unstable();
                (clause_col[ci], s)
            })
            .collect();
        let new_var = rank_signatures(&var_sig);
        let new_clause = rank_signatures(&clause_sig);
        let new_classes = class_count(&new_var) + class_count(&new_clause);
        var_col.copy_from_slice(&new_var);
        clause_col.copy_from_slice(&new_clause);
        if new_classes <= classes {
            return;
        }
        classes = new_classes;
    }
}

/// Each signature's rank among the distinct signatures, in sorted order.
fn rank_signatures(sigs: &[Signature]) -> Vec<u32> {
    let mut distinct: Vec<&Signature> = sigs.iter().collect();
    distinct.sort_unstable();
    distinct.dedup();
    sigs.iter()
        .map(|s| {
            distinct
                .binary_search(&s)
                .expect("every signature is listed") as u32
        })
        .collect()
}

fn class_count(colours: &[u32]) -> usize {
    let mut sorted = colours.to_vec();
    sorted.sort_unstable();
    sorted.dedup();
    sorted.len()
}
