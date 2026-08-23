//! Writing the split: the per-component CNFs and vtrees, the retained
//! candidate sets, and the `components.json` that indexes them.
//!
//! Every file named in a manifest this module produces is a file it wrote.

use std::path::{Path, PathBuf};

use super::{
    CANDIDATES_DIR, COMPONENTS_DIR, COMPONENTS_FORMAT_TAG, COMPONENTS_JSON_NAME, CandidateEntry,
    ComponentEntry, ComponentPaths, ComponentWriteOptions, ComponentsManifest, SelectionEntry,
    TreeDecompositionSummary,
};
use crate::bundle::{
    DotFor, REDUCED_CNF_NAME, VTREE_NAME, ensure_dir, to_json_pretty, write_file, write_vtree_files,
};
use crate::candidates::CandidateSet;
use crate::cnf::{CnfFormula, DimacsHeader, Reduced, ShowSet, write_dimacs};
use crate::component::{LocalView, VtreeBuild, local_view};
use crate::error::VitriError;
use crate::vtree::VarId;

/// Write the per-component CNFs, vtrees and the manifest into `dir`.
///
/// `build.components` is `Some` when the formula split and `None` when it did
/// not; `None` still produces a manifest — one entry, the identity map,
/// pointing at the top-level files — so a consumer never needs a second code
/// path for "it happened to be connected". The whole build is passed rather
/// than its pieces so the component descriptors, their selection records and
/// their candidate sets cannot be handed in mismatched.
///
/// `show_reduced` is the projection show set over the REDUCED formula (as
/// recorded in `preprocess.json`), or `None` for a non-projected instance.
///
/// Each component's own CNF and show set are derived here the same way the
/// splitter derived the ones it built the component's vtree over: the same pure
/// function on the same clause indices, so the files and the map cannot
/// disagree about what LOCAL numbering means. The values are re-derived rather
/// than carried across from the build, for the reason
/// [`ComponentVtree`](crate::component::ComponentVtree) records.
///
/// `options` adds the optional extras (see [`ComponentWriteOptions`]) — this is
/// the one place any per-component or per-candidate file is written, so an
/// extra is emitted for every vtree here or for none.
///
/// # Errors
///
/// [`VitriError::Io`] naming the file or directory that could not be written,
/// and [`VitriError::Mismatch`] for a component vtree that does not span its
/// component's variables — refused before anything is written rather than
/// exported as a bundle whose parts describe different formulas.
pub fn write_components(
    dir: &Path,
    reduced: &CnfFormula,
    build: &VtreeBuild,
    show_reduced: Option<&ShowSet<Reduced>>,
    options: ComponentWriteOptions,
) -> Result<(ComponentsManifest, ComponentPaths), VitriError> {
    // Ranks 1.. of every candidate set are ordered by one metric, fixed by the
    // counting mode — read off the candidate sets so the manifest can't claim
    // a ranking they weren't actually sorted by.
    let rank_metric = build
        .candidate_sets
        .iter()
        .find(|b| !b.is_empty())
        .map(|b| b.metric);
    let mut cand_files: Vec<PathBuf> = Vec::new();
    // One mask per FORMULA, not per component: every branch below restricts
    // this same reduced-formula mask rather than rebuilding it.
    let reduced_mask = show_reduced.map(|s| s.mask(reduced.num_vars));

    let Some(comps) = build.components.as_deref() else {
        // Single component: it is the reduced formula, with the identity map —
        // its candidate set is the build's single candidate set.
        let cnf = REDUCED_CNF_NAME.to_string();
        let vtree_file = VTREE_NAME.to_string();
        let set = build.candidate_sets.first();
        // Rank 0 is the top-level vtree, whose own `.dot` belongs to whoever
        // wrote it; only the runners-up here get one, against the same formula.
        let dot = DotFor::when(options.dot, reduced, reduced_mask.as_ref());
        // LOCAL `i` IS REDUCED `i` here, and the show set is restricted through
        // that map rather than reinterpreted, so this branch and the split one
        // below get their local sets the same way.
        let local_to_reduced: Vec<VarId> = (0..reduced.num_vars).map(VarId).collect();
        let entry = ComponentEntry {
            local_to_reduced_dimacs: local_to_reduced_dimacs(&local_to_reduced),
            show_vars_local_dimacs: reduced_mask.as_ref().map(|m| m.restrict(&local_to_reduced)),
            selection: selection_entry(build.selections.first()),
            vtree_candidates: match set {
                Some(b) => write_candidate_set(dir, 0, b, &vtree_file, &mut cand_files, dot)?,
                None => Vec::new(),
            },
            cnf,
            vtree: vtree_file,
        };
        let manifest = ComponentsManifest {
            format: COMPONENTS_FORMAT_TAG.to_string(),
            free_vars_reduced_dimacs: Vec::new(),
            candidate_rank_metric: rank_metric,
            components: vec![entry],
        };
        let path = write_manifest(dir, &manifest)?;
        return Ok((
            manifest,
            ComponentPaths {
                manifest: path,
                files: Vec::new(),
                candidates: cand_files,
            },
        ));
    };

    let comp_dir = dir.join(COMPONENTS_DIR);
    ensure_dir(&comp_dir)?;

    // REDUCED ids claimed by some component, 0-based here (internal indexing,
    // not DIMACS); whatever's left is free. Built here rather than re-derived
    // from the clause list so it agrees with the split by construction.
    let mut claimed = vec![false; reduced.num_vars as usize];
    let mut entries = Vec::with_capacity(comps.len());
    let mut files = Vec::with_capacity(comps.len() * 3);

    for (index, cv) in comps.iter().enumerate() {
        let LocalView {
            formula: sub,
            show: show_local,
            local_to_outer: local_to_reduced,
        } = local_view(reduced, &cv.clause_indices, reduced_mask.as_ref());
        // `build` and `reduced` arrive as separate arguments, so a caller can
        // pair a build with a formula it was not made from. That is a bad call,
        // not a broken invariant, and it deserves an error rather than a panic
        // in the middle of writing a half-finished bundle.
        if cv.vtree.num_leaves() != sub.num_vars {
            return Err(VitriError::mismatch(format!(
                "component {index} vtree has {} leaves but its CNF has {} variables; \
                 the build does not belong to this formula",
                cv.vtree.num_leaves(),
                sub.num_vars,
            )));
        }
        for v in &local_to_reduced {
            claimed[v.idx()] = true;
        }

        // Every picture of this component's vtrees is annotated against the
        // component's own CNF and its own share of the show set — the space its
        // vtree is built over.
        let show_mask = show_local.as_ref().map(|s| s.mask(sub.num_vars));
        let dot = DotFor::when(options.dot, &sub, show_mask.as_ref());

        let stem = format!("comp{index:03}");
        let cnf_rel = format!("{COMPONENTS_DIR}/{stem}.cnf");
        let vtree_rel = format!("{COMPONENTS_DIR}/{stem}.vtree");

        let cnf_path = dir.join(&cnf_rel);
        // A component file carries only its own show set — the mode header and
        // weight table belong to the whole instance; repeating them beside a
        // partial clause set would describe a counting problem this file isn't.
        write_dimacs(
            &sub,
            &DimacsHeader {
                show: show_local.as_ref(),
                ..Default::default()
            },
            &cnf_path,
        )?;
        let (vtree_path, dot_path) = write_vtree_files(dir.join(&vtree_rel), &cv.vtree, dot)?;
        files.extend([cnf_path, vtree_path]);
        files.extend(dot_path);

        // `candidate_sets` is aligned 1:1 with `comps`, and an entry with
        // nothing retained is empty rather than missing — but a build is a
        // caller's argument, so this reads it for what it is rather than
        // indexing on the strength of the contract.
        let vtree_candidates = match build.candidate_sets.get(index) {
            Some(b) => write_candidate_set(dir, index, b, &vtree_rel, &mut cand_files, dot)?,
            None => Vec::new(),
        };

        entries.push(ComponentEntry {
            local_to_reduced_dimacs: local_to_reduced_dimacs(&local_to_reduced),
            show_vars_local_dimacs: show_local,
            // `selections` is aligned 1:1 with `comps`, so this is the record
            // of THIS component's own construction, not the whole build's.
            selection: selection_entry(build.selections.get(index)),
            vtree_candidates,
            cnf: cnf_rel,
            vtree: vtree_rel,
        });
    }

    let manifest = ComponentsManifest {
        format: COMPONENTS_FORMAT_TAG.to_string(),
        free_vars_reduced_dimacs: (0..reduced.num_vars)
            .filter(|&v| !claimed[v as usize])
            .map(|v| VarId(v).to_dimacs() as u32)
            .collect(),
        candidate_rank_metric: rank_metric,
        components: entries,
    };
    let manifest_path = write_manifest(dir, &manifest)?;
    Ok((
        manifest,
        ComponentPaths {
            manifest: manifest_path,
            files,
            candidates: cand_files,
        },
    ))
}

/// A component's LOCAL→REDUCED variable map as the manifest carries it: 1-based
/// at both ends, so entry `local - 1` is the reduced DIMACS id that local
/// variable `local` stands for.
fn local_to_reduced_dimacs(local_to_reduced: &[VarId]) -> Vec<u32> {
    local_to_reduced
        .iter()
        .map(|v| v.to_dimacs() as u32)
        .collect()
}

/// Project one component's [`SelectionRecord`](crate::spec::SelectionRecord)
/// onto the manifest — the whole of the export side of that record, so what the
/// build collected and what the bundle publishes cannot drift apart.
///
/// The bag metadata is summarised here rather than copied: its per-variable map
/// is component-LOCAL and as long as the component, and a consumer holding the
/// component's own CNF and vtree can already see where every variable went.
fn selection_entry(record: Option<&crate::spec::SelectionRecord>) -> Option<SelectionEntry> {
    let record = record?;
    Some(SelectionEntry {
        winning_spec: record.winning_spec.clone()?,
        tree_decomposition: record.td_meta.as_deref().map(|m| TreeDecompositionSummary {
            num_bags: m.num_bags(),
            max_bag_size: m.max_bag_size(),
        }),
    })
}

/// Write one component's runner-up vtrees and return its manifest entries.
///
/// `selected_vtree` is the component's own vtree file path, already written by
/// the caller. Rank 0 reuses it instead of writing a copy: the selected vtree
/// is by definition the same tree, and a consumer that can't tell the two
/// apart is exactly the confusion the candidate set exists to remove.
///
/// Returns an empty vec and writes nothing — not even a `candidates/`
/// directory — for an empty candidate set, so a caller who never asks for one
/// sees no trace of this mechanism.
///
/// `dot` pictures each runner-up against the formula the candidate set was
/// scored on. Rank 0 gets none for the same reason it gets no `.vtree`: it's
/// the file the caller already wrote, and its picture belongs beside that one.
fn write_candidate_set(
    dir: &Path,
    index: usize,
    set: &CandidateSet,
    selected_vtree: &str,
    files: &mut Vec<PathBuf>,
    dot: Option<DotFor<'_>>,
) -> Result<Vec<CandidateEntry>, VitriError> {
    if set.is_empty() {
        return Ok(Vec::new());
    }
    let mut out = Vec::with_capacity(set.candidates.len());
    let mut made_dir = false;
    for (rank, cand) in set.candidates.iter().enumerate() {
        let vtree_rel = if rank == 0 {
            selected_vtree.to_string()
        } else {
            if !made_dir {
                ensure_dir(&dir.join(CANDIDATES_DIR))?;
                made_dir = true;
            }
            let stem = format!("comp{index:03}.rank{rank:02}");
            let vtree_rel = format!("{CANDIDATES_DIR}/{stem}.vtree");
            let (vtree_path, dot_path) = write_vtree_files(dir.join(&vtree_rel), &cand.vtree, dot)?;
            files.push(vtree_path);
            files.extend(dot_path);
            vtree_rel
        };
        debug_assert!(
            (rank == 0) == cand.selected,
            "the first entry of an emitted candidate set must be the selected vtree, and only it",
        );
        out.push(CandidateEntry {
            built_by: cand.built_by.clone(),
            vtree: vtree_rel,
            scores: cand.scores,
        });
    }
    Ok(out)
}

fn write_manifest(dir: &Path, manifest: &ComponentsManifest) -> Result<PathBuf, VitriError> {
    ensure_dir(dir)?;
    let path = dir.join(COMPONENTS_JSON_NAME);
    write_file(&path, to_json_pretty(manifest))?;
    Ok(path)
}
