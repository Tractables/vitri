//! The text `-h` / `--help` prints.
//!
//! Every list here is rendered from a table something else reads: the options
//! from [`OPTIONS`], which the parser also resolves each argument against, and
//! the `--vtree` grammar from the spec vocabulary the parser matches against.
//! A flag or a spec name added to one of those tables is printed here by the
//! same change that makes the binary accept it.

use vitri::bundle;
use vitri::cnf::Mode;
use vitri::config::ComponentPolicy;
use vitri::spec::DEFAULT_VTREE_SPEC;

use super::{OPTIONS, Opt, OptKey};

impl Opt {
    /// The left column of this option's help entry: the spellings and the value
    /// placeholder, indented under `OPTIONS:`.
    fn head(&self) -> String {
        let short = self
            .short
            .map_or_else(|| "    ".to_string(), |s| format!("{s}, "));
        let value = self.value.map_or_else(String::new, |v| format!(" {v}"));
        format!("    {short}{}{value}", self.long)
    }
}

/// The column every blurb in the `OPTIONS:` block starts at.
const BLURB_COL: usize = 25;

/// What `--help` says about `--vtree`: the grammar, every base with the
/// parameters it takes, and every parameter with its values and its default.
///
/// Rendered entirely from the tables the parser matches against
/// ([`vitri::spec::vtree_spec_bases`], [`vitri::spec::spec_param_docs`]), so
/// the help cannot omit a base or a key the parser accepts, nor advertise one
/// it does not. That completeness is the point: a reader of `--help` alone can
/// write any spec this crate will build.
fn vtree_blurb() -> String {
    let bases = vitri::spec::vtree_spec_bases();
    // Every key any base takes, in grammar order, described once below the base
    // list rather than repeated under each base that accepts it. Two families
    // can spell different parameters with the same word — `root=` names a bag
    // of a decomposition on one and where an embedding's tree is rooted on
    // another — so a row is the same row only when its whole description is.
    let mut keys: Vec<vitri::spec::SpecParamDoc> = Vec::new();
    let mut lines = vec![
        format!("Vtree construction strategy. Default: {DEFAULT_VTREE_SPEC}."),
        "A spec is <base>[:key=value[,key=value]...] — every".to_string(),
        "parameter is written with its key, at most once, and a".to_string(),
        "key the base does not take is refused.".to_string(),
        String::new(),
        "Bases, with the parameters each takes:".to_string(),
    ];
    for base in &bases {
        let docs = vitri::spec::spec_param_docs(base);
        for d in &docs {
            if !keys.contains(d) {
                keys.push(d.clone());
            }
        }
        let taken = if docs.is_empty() {
            "no parameters".to_string()
        } else {
            docs.iter()
                .map(|d| format!("{}=", d.key))
                .collect::<Vec<_>>()
                .join(" ")
        };
        lines.push(format!("  {base:<28}{taken}"));
    }
    lines.push(String::new());
    lines.push("Parameters:".to_string());
    for k in &keys {
        lines.push(format!("  {}={}", k.key, k.values));
        lines.push(format!("      {}", k.what));
        lines.push(format!("      default: {}", k.default));
    }
    lines.join("\n")
}

impl OptKey {
    /// What `--help` says about this option, wrapped by hand at the width the
    /// blurb column leaves.
    ///
    /// Every vocabulary quoted here comes from the table the parser itself
    /// matches against, so a mode, a policy or a construction added there is
    /// offered without a second edit. The lists arrive as the runs their lines
    /// hold — a name added to a table joins that list's last run.
    fn blurb(self) -> String {
        match self {
            OptKey::OutDir => "Directory to write the bundle into (created if\n\
                 missing). Required."
                .to_string(),
            OptKey::Mode => {
                let modes: Vec<&str> = Mode::names().collect();
                let wrap = modes.len().min(4);
                format!(
                    "What preprocessing must preserve: {},\n\
                     or {}. Default: detected from the input's own\n\
                     headers. Stating it WINS over them; a declaration the\n\
                     mode does not use is reported and ignored. `compile`\n\
                     preserves the FUNCTION, not a count — only stages the\n\
                     record can undo run, so it reduces less than any\n\
                     counting mode.",
                    modes[..wrap].join(", "),
                    modes[wrap..].join(", "),
                )
            }
            OptKey::Vtree => vtree_blurb(),
            OptKey::BudgetMs => "Wall-clock budget hint, in milliseconds, for the whole\n\
                 run. Vtree construction gets a share of it and hands\n\
                 back the best candidate it has when that share runs\n\
                 out, so a larger budget can yield a different (better)\n\
                 vtree. Default: unbounded."
                .to_string(),
            OptKey::Components => format!(
                "`{split}` (default) splits the reduced formula into its\n\
                 independent sub-problems and builds a vtree for each.\n\
                 `{whole}` builds one vtree over everything. {comps} is\n\
                 written either way.",
                split = ComponentPolicy::Split.token(),
                whole = ComponentPolicy::Whole.token(),
                comps = bundle::components::COMPONENTS_JSON_NAME,
            ),
            OptKey::Candidates => format!(
                "Also emit the next-best vtrees the portfolio built and\n\
                 scored on its way to picking the winner, N in total\n\
                 (default 1 = winner only, max {maxcands}). They were\n\
                 already built during the search, so this only writes\n\
                 more files. Ranked best-first in {comps}, each with\n\
                 the scores it was ranked on and the construction that\n\
                 produced it; two constructions that converged on the\n\
                 same tree are listed as one entry. Only the portfolio\n\
                 (`{DEFAULT_VTREE_SPEC}`) has a candidate set.",
                maxcands = vitri::candidates::MAX_CANDIDATES,
                comps = bundle::components::COMPONENTS_JSON_NAME,
            ),
            OptKey::Dot => "Also write a Graphviz `.dot` beside every `.vtree` this\n\
                 run emits, same stem — the whole-formula one, and each\n\
                 component and candidate vtree. Every node is coloured by\n\
                 its clause load; internal nodes also carry the label\n\
                 `c=<clause load> w=<context width>`, measured against\n\
                 the CNF that vtree serves. Render one with\n\
                 `dot -Tsvg vtree.dot > vtree.svg`."
                .to_string(),
            OptKey::NoArjun => "Skip the Arjun stage. Weaker preprocessing, less\n\
                 time spent. `compile` has no Arjun stage."
                .to_string(),
            OptKey::NoSimplify => "Skip this crate's own simplify chain: CaDiCaL clause\n\
                 simplification, equivalence detection, backbone and\n\
                 equivalence probing, backbone and dead-variable\n\
                 stripping, equivalence reduction, gate detection, DVE.\n\
                 `pmc` and `pwmc` have no simplify chain, and refuse\n\
                 this flag for the same reason."
                .to_string(),
            OptKey::Help => "Print this message.".to_string(),
            OptKey::Version => "Print the version and exit.".to_string(),
        }
    }
}

/// The `OPTIONS:` block, rendered from [`OPTIONS`].
///
/// Each blurb starts at [`BLURB_COL`]; an entry whose spellings reach that
/// column takes the next line for its blurb instead, so the column holds however
/// long an option's name is.
fn options_block() -> String {
    let mut out = String::new();
    for opt in OPTIONS {
        let head = opt.head();
        let blurb = opt.key.blurb();
        let mut lines = blurb.lines();
        if head.len() < BLURB_COL {
            let first = lines.next().unwrap_or_default();
            out.push_str(&format!("{head:<width$}{first}\n", width = BLURB_COL));
        } else {
            out.push_str(&format!("{head}\n"));
        }
        for line in lines {
            out.push_str(&format!("{:width$}{line}\n", "", width = BLURB_COL));
        }
    }
    out
}

/// The usage text, as `-h` / `--help` prints it.
pub(super) fn help() -> String {
    format!(
        "\
vitri — preprocess a CNF and build a vtree for it.

Writes everything a knowledge compiler — d-DNNF, SDD, or tree decision diagram
(TDD) — needs to compile the instance itself: the reduced formula, the
arithmetic to lift a model count over it back to the original CNF, and the
selected vtree.

USAGE:
    vitri <input.cnf> --out-dir <DIR> [OPTIONS]

ARGS:
    <input.cnf>          DIMACS CNF. The Model Counting Competition (MCC)
                         `c t <track>` header and the `c p show` / `c p weight`
                         lines are understood.

OPTIONS:
{options}
OUTPUT (in <DIR>):
    {reduced:<17}The reduced formula, DIMACS. Self-describing: it carries
                     its own `c t` track header (none under `--mode compile`,
                     which is not a track), its own `c p show` line (reduced
                     ids) and its own `c p weight` lines (reduced ids, exact
                     rationals), so the file states the problem it belongs to.
    {record:<17}How to get back to the original: the count lift (a power of
                     two and an exact rational), the reduced->original variable
                     map, forced literals, free variables, the show set and the
                     reduced weights. Under --mode compile it also carries the
                     original->reduced map, which names EVERY original variable
                     and is what makes that mode's preprocessing undoable.
    {vtree:<17}The vtree, standard SDD text format. Variables are 1-BASED
                     DIMACS and number the variables of {reduced}, not of the
                     input.
    {comps:<17}The independent sub-problems of {reduced}: for each, its
                     LOCAL<->reduced variable map and the files below. Always
                     written, even for one component.
    {cdir:<17}Per component: compNNN.cnf (LOCAL 1-based DIMACS),
                     compNNN.vtree (LOCAL 1-based). Absent when the formula
                     has one component — {comps} then points at the
                     whole-formula files.
    {altdir:<17}Only with --candidates: the RUNNER-UP vtrees, as
                     compNNN.rankRR.vtree in the same LOCAL space as their
                     component. Entry 0 is the selected vtree and is not
                     copied here — its entry in {comps} points at the
                     component's own vtree file.

EXIT STATUS:
    0                    The bundle was written.
    1                    The invocation was fine and the work failed.
    2                    The invocation was wrong — a bad argument, a flag
                         combination that would do nothing, or a `VITRI_*`
                         variable this crate cannot use. Nothing ran, and
                         nothing was written.

EXAMPLE:
    vitri instance.cnf -o bundle/ --vtree {DEFAULT_VTREE_SPEC} --budget-ms 60000

    A count over bundle/{reduced} — taken under that file's own mode, show set
    and weights — is lifted to a count over instance.cnf by multiplying by
    2^count_lift_pow2 and by weight_lift (an exact `num/den` rational),
    both from bundle/{record}. One of the two is always inert, so a
    consumer applies both unconditionally. Counting the components separately
    instead: multiply their counts together, then by 2 to the power of the
    number of entries in free_vars_reduced_dimacs from {comps}, then
    apply the lift.
",
        options = options_block(),
        reduced = bundle::REDUCED_CNF_NAME,
        record = bundle::PREPROCESS_RECORD_NAME,
        vtree = bundle::VTREE_NAME,
        comps = bundle::components::COMPONENTS_JSON_NAME,
        cdir = format!("{}/", bundle::components::COMPONENTS_DIR),
        altdir = format!("{}/", bundle::components::CANDIDATES_DIR),
    )
}
