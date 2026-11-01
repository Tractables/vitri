//! Writing the split: the per-component CNFs and vtrees, the retained
//! candidate sets, and the `components.json` that indexes them.
//!
//! Every file named in a manifest this module produces is a file it wrote.

use std::path::{Path, PathBuf};

use super::{
    CANDIDATES_DIR, COMPONENTS_DIR, COMPONENTS_JSON_NAME, CandidateEntry, ComponentEntry,
    ComponentPaths, ComponentWriteOptions, ComponentsManifest, SelectionEntry,
    TreeDecompositionSummary,
};
use crate::bundle::{
    ComponentFiles, DotFor, REDUCED_CNF_NAME, Sink, VTREE_NAME, to_json_pretty, write_vtree_files,
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
) -> Result<ComponentFiles, VitriError> {
    write_components_to(&mut Sink::at(dir), reduced, build, show_reduced, options)
}

/// [`write_components`] into either destination a [`Sink`] names.
pub(in crate::bundle) fn write_components_to(
    sink: &mut Sink<'_>,
    reduced: &CnfFormula,
    build: &VtreeBuild,
    show_reduced: Option<&ShowSet<Reduced>>,
    options: ComponentWriteOptions,
) -> Result<ComponentFiles, VitriError> {
    // Ranks 1.. of every candidate set are ordered by one metric, fixed by the
    // counting mode — read off the candidate sets so the manifest can't claim
    // a ranking they weren't actually sorted by.
    let rank_metric = build
        .candidate_sets
        .iter()
        .find(|b| !b.is_empty())
        .map(|b| b.metric);
    // One mask per FORMULA, not per component: every branch below restricts
    // this same reduced-formula mask rather than rebuilding it.
    let reduced_mask = show_reduced.map(|s| s.mask(reduced.num_vars()));

    let Some(comps) = build.components.as_deref() else {
        return write_single_component_manifest(
            sink,
            reduced,
            build,
            reduced_mask.as_ref(),
            rank_metric,
            options,
            Vec::new(),
        );
    };

    sink.dir(COMPONENTS_DIR)?;

    // REDUCED variables claimed by some component, indexed by `VarId::idx`;
    // whatever's left is free. Built here rather than re-derived from the
    // clause list so it agrees with the split by construction.
    let mut batch = ComponentBatch {
        sink,
        reduced,
        build,
        reduced_mask: reduced_mask.as_ref(),
        options,
        claimed: vec![false; reduced.num_vars() as usize],
        files: Vec::with_capacity(comps.len() * 3),
        cand_files: Vec::new(),
    };
    let mut entries = Vec::with_capacity(comps.len());
    for (index, cv) in comps.iter().enumerate() {
        entries.push(batch.write_one(index, cv)?);
    }

    let manifest = ComponentsManifest::new(
        unclaimed_vars(reduced.num_vars(), &batch.claimed),
        rank_metric,
        entries,
    );
    finish_component_files(batch.sink, manifest, batch.files, batch.cand_files)
}

fn finish_component_files(
    sink: &mut Sink<'_>,
    manifest: ComponentsManifest,
    files: Vec<PathBuf>,
    candidates: Vec<PathBuf>,
) -> Result<ComponentFiles, VitriError> {
    let manifest_path = write_manifest(sink, &manifest)?;
    Ok(ComponentFiles {
        manifest,
        paths: ComponentPaths {
            manifest: manifest_path,
            files,
            candidates,
        },
    })
}

struct ComponentBatch<'a, 's> {
    sink: &'a mut Sink<'s>,
    reduced: &'a CnfFormula,
    build: &'a VtreeBuild,
    reduced_mask: Option<&'a crate::cnf::ShowMask>,
    options: ComponentWriteOptions,
    claimed: Vec<bool>,
    files: Vec<PathBuf>,
    cand_files: Vec<PathBuf>,
}

impl ComponentBatch<'_, '_> {
    /// A component file carries only its own show set — the mode header and
    /// weight table belong to the whole instance; repeating them beside a
    /// partial clause set would describe a counting problem this file isn't.
    fn write_one(
        &mut self,
        index: usize,
        cv: &crate::component::ComponentVtree,
    ) -> Result<ComponentEntry, VitriError> {
        let LocalView {
            formula: sub,
            show: show_local,
            local_to_outer: local_to_reduced,
        } = local_view(self.reduced, &cv.clause_indices, self.reduced_mask);
        for v in &local_to_reduced {
            self.claimed[v.idx()] = true;
        }

        // Every picture of this component's vtrees is annotated against the
        // component's own CNF and its own share of the show set — the space its
        // vtree is built over.
        let show_mask = show_local.as_ref().map(|s| s.mask(sub.num_vars()));
        let dot = DotFor::when(self.options.dot, &sub, show_mask.as_ref());

        let stem = format!("comp{index:03}");
        let cnf_rel = format!("{COMPONENTS_DIR}/{stem}.cnf");
        let vtree_rel = format!("{COMPONENTS_DIR}/{stem}.vtree");

        let header = DimacsHeader {
            show: show_local.as_ref(),
            ..Default::default()
        };
        let cnf_path = self
            .sink
            .file(&cnf_rel, |w| write_dimacs(&sub, &header, w))?;
        let (vtree_path, dot_path) = write_vtree_files(self.sink, &vtree_rel, &cv.vtree, dot)?;
        self.files.extend([cnf_path, vtree_path]);
        self.files.extend(dot_path);

        let vtree_candidates = write_candidate_set(
            self.sink,
            index,
            self.build.candidate_sets.get(index),
            &vtree_rel,
            &mut self.cand_files,
            dot,
        )?;

        Ok(build_component_entry(
            self.build,
            index,
            &local_to_reduced,
            show_local,
            vtree_candidates,
            cnf_rel,
            vtree_rel,
        ))
    }
}

fn unclaimed_vars(num_vars: u32, claimed: &[bool]) -> Vec<u32> {
    (0..num_vars as usize)
        .filter(|&v| !claimed[v])
        .map(|v| VarId::from_idx(v).get())
        .collect()
}

fn write_single_component_manifest(
    sink: &mut Sink<'_>,
    reduced: &CnfFormula,
    build: &VtreeBuild,
    reduced_mask: Option<&crate::cnf::ShowMask>,
    rank_metric: Option<crate::candidates::CandidateRankMetric>,
    options: ComponentWriteOptions,
    mut cand_files: Vec<PathBuf>,
) -> Result<ComponentFiles, VitriError> {
    let dot = DotFor::when(options.dot, reduced, reduced_mask);
    let all_clauses: Vec<usize> = (0..reduced.clauses().len()).collect();
    let LocalView {
        show: show_local,
        local_to_outer: local_to_reduced,
        ..
    } = local_view(reduced, &all_clauses, reduced_mask);
    let mut claimed = vec![false; reduced.num_vars() as usize];
    for v in &local_to_reduced {
        claimed[v.idx()] = true;
    }
    let free = unclaimed_vars(reduced.num_vars(), &claimed);
    let vtree_candidates = write_candidate_set(
        sink,
        0,
        build.candidate_sets.first(),
        VTREE_NAME,
        &mut cand_files,
        dot,
    )?;
    let entry = build_component_entry(
        build,
        0,
        &local_to_reduced,
        show_local,
        vtree_candidates,
        REDUCED_CNF_NAME.to_owned(),
        VTREE_NAME.to_owned(),
    );
    let manifest = ComponentsManifest::new(free, rank_metric, vec![entry]);
    let manifest_path = write_manifest(sink, &manifest)?;
    Ok(ComponentFiles {
        manifest,
        paths: ComponentPaths {
            manifest: manifest_path,
            files: Vec::new(),
            candidates: cand_files,
        },
    })
}

fn build_component_entry(
    build: &VtreeBuild,
    index: usize,
    local_to_reduced: &[VarId],
    show_vars_local_dimacs: Option<ShowSet<crate::cnf::Local>>,
    vtree_candidates: Vec<CandidateEntry>,
    cnf: String,
    vtree: String,
) -> ComponentEntry {
    ComponentEntry {
        local_to_reduced_dimacs: local_to_reduced_dimacs(local_to_reduced),
        show_vars_local_dimacs,
        // `selections` is aligned 1:1 with `comps`, so this is the record
        // of THIS component's own construction, not the whole build's.
        selection: selection_entry(build.selections.get(index)),
        vtree_candidates,
        cnf,
        vtree,
    }
}

/// A component's LOCAL→REDUCED variable map as the manifest carries it: entry
/// `local - 1` is the reduced DIMACS id that local variable `local` stands for.
fn local_to_reduced_dimacs(local_to_reduced: &[VarId]) -> Vec<u32> {
    local_to_reduced.iter().map(|v| v.get()).collect()
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
            treewidth: m.treewidth(),
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
    sink: &mut Sink<'_>,
    index: usize,
    set: Option<&CandidateSet>,
    selected_vtree: &str,
    files: &mut Vec<PathBuf>,
    dot: Option<DotFor<'_>>,
) -> Result<Vec<CandidateEntry>, VitriError> {
    let Some(set) = set.filter(|s| !s.is_empty()) else {
        return Ok(Vec::new());
    };
    let mut out = Vec::with_capacity(set.candidates.len());
    let mut made_dir = false;
    for (rank, cand) in set.candidates.iter().enumerate() {
        let vtree_rel = if rank == 0 {
            selected_vtree.to_string()
        } else {
            if !made_dir {
                sink.dir(CANDIDATES_DIR)?;
                made_dir = true;
            }
            let stem = format!("comp{index:03}.rank{rank:02}");
            let vtree_rel = format!("{CANDIDATES_DIR}/{stem}.vtree");
            let (vtree_path, dot_path) = write_vtree_files(sink, &vtree_rel, &cand.vtree, dot)?;
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

fn write_manifest(
    sink: &mut Sink<'_>,
    manifest: &ComponentsManifest,
) -> Result<PathBuf, VitriError> {
    sink.dir("")?;
    sink.file(COMPONENTS_JSON_NAME, |w| {
        w.write_all(to_json_pretty(manifest).as_bytes())
    })
}
