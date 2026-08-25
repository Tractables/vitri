//! `vitri` — turn a raw CNF into a reduced CNF plus a good vtree, for a
//! circuit compiler or model counter to consume.
//!
//! This binary is a thin shell: every capability it exposes is a call into the
//! library's public API ([`vitri::bundle`] for the preprocessing and the
//! export, [`vitri::component`] for vtree construction), never a second
//! implementation. Anything reachable from the command line is reachable from
//! the API — the flags parsed here become fields of one
//! [`RunConfig`](vitri::config::RunConfig), which is the whole input
//! to both calls.
//!
//! See `docs/bundle.md` for the output-file contract, `docs/preprocessing.md`
//! for what the consumer is responsible for, and `docs/env.md` for the `VITRI_*`
//! variables this binary reads.

use std::fs::File;
use std::io::BufReader;
use std::path::PathBuf;
use std::process::exit;

use vitri::VitriError;
use vitri::bundle;
use vitri::candidates;
use vitri::cnf::{CnfFormula, Mode};
use vitri::component::VtreeBuild;
use vitri::config::{ComponentPolicy, RunConfig};
use vitri::decompose::SelectionCtx;
use vitri::spec::DEFAULT_VTREE_SPEC;

/// The accepted values a rejection ends with, in the tool's one phrasing:
/// `a, b or c`.
///
/// Every closed vocabulary the command line takes hands over its own name list,
/// so the message offers what the parser accepts rather than a copy of it.
fn one_of(names: impl Iterator<Item = &'static str>) -> String {
    let names: Vec<&str> = names.collect();
    match names.split_last() {
        Some((last, [])) => (*last).to_string(),
        Some((last, rest)) => format!("{} or {last}", rest.join(", ")),
        None => String::new(),
    }
}

/// Which option a row of [`OPTIONS`] is.
///
/// The parser dispatches on this, exhaustively, so a row added without an arm
/// to answer it is a build error rather than a flag `--help` offers and the
/// parser rejects.
#[derive(Clone, Copy, PartialEq, Eq)]
enum OptKey {
    OutDir,
    Mode,
    Vtree,
    BudgetMs,
    Components,
    Candidates,
    Dot,
    NoArjun,
    NoSimplify,
    Help,
}

/// One option the command line takes: how it is spelled, whether it carries a
/// value, and — through [`OptKey::blurb`] — what `--help` says about it.
struct Opt {
    key: OptKey,
    /// The short spelling, for the two options that have one.
    short: Option<&'static str>,
    /// The long spelling, which is also how a message names the option.
    long: &'static str,
    /// The value placeholder `--help` prints after the spellings; `None` for a
    /// switch that takes no value.
    value: Option<&'static str>,
}

/// Every option, in the order `--help` lists them.
///
/// Read twice: [`help`] renders the `OPTIONS:` block from it, and [`parse_argv`]
/// resolves each argument against it before dispatching. So an option that is
/// not here is neither offered nor accepted, and the help text cannot describe a
/// grammar the parser does not have.
const OPTIONS: &[Opt] = &[
    Opt {
        key: OptKey::OutDir,
        short: Some("-o"),
        long: "--out-dir",
        value: Some("<DIR>"),
    },
    Opt {
        key: OptKey::Mode,
        short: None,
        long: "--mode",
        value: Some("<MODE>"),
    },
    Opt {
        key: OptKey::Vtree,
        short: None,
        long: "--vtree",
        value: Some("<SPEC>"),
    },
    Opt {
        key: OptKey::BudgetMs,
        short: None,
        long: "--budget-ms",
        value: Some("<N>"),
    },
    Opt {
        key: OptKey::Components,
        short: None,
        long: "--components",
        value: Some("<MODE>"),
    },
    Opt {
        key: OptKey::Candidates,
        short: None,
        long: "--candidates",
        value: Some("<N>"),
    },
    Opt {
        key: OptKey::Dot,
        short: None,
        long: "--dot",
        value: None,
    },
    Opt {
        key: OptKey::NoArjun,
        short: None,
        long: "--no-arjun",
        value: None,
    },
    Opt {
        key: OptKey::NoSimplify,
        short: None,
        long: "--no-simplify",
        value: None,
    },
    Opt {
        key: OptKey::Help,
        short: Some("-h"),
        long: "--help",
        value: None,
    },
];

/// The column every blurb in the `OPTIONS:` block starts at.
const BLURB_COL: usize = 25;

impl Opt {
    /// Whether `arg` is this option, under either spelling.
    fn matches(&self, arg: &str) -> bool {
        self.long == arg || self.short == Some(arg)
    }

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
            let same = |k: &vitri::spec::SpecParamDoc| {
                k.key == d.key && k.values == d.values && k.default == d.default && k.what == d.what
            };
            if !keys.iter().any(same) {
                keys.push(vitri::spec::SpecParamDoc {
                    key: d.key,
                    values: d.values.clone(),
                    default: d.default,
                    what: d.what,
                });
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
                 (default 1 = winner only, max {maxcands}). They are\n\
                 free: they were already constructed. Ranked\n\
                 best-first in {comps}, each with the scores it was\n\
                 ranked on and the construction that produced it; two\n\
                 constructions that converged on the same tree are\n\
                 listed as one entry. Only the portfolio\n\
                 (`{DEFAULT_VTREE_SPEC}`) has a candidate set.",
                maxcands = vitri::candidates::MAX_CANDIDATES,
                comps = bundle::components::COMPONENTS_JSON_NAME,
            ),
            OptKey::Dot => "Also write a Graphviz `.dot` beside every `.vtree` this\n\
                 run emits, same stem — the whole-formula one, and each\n\
                 component and candidate vtree. Every node is coloured by\n\
                 its clause load and labelled `c=<clause load>\n\
                 w=<context width>`, measured against the CNF that vtree\n\
                 serves. Render one with\n\
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
fn help() -> String {
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
                     its own `c t` track header, its own `c p show` line (reduced
                     ids) and its own `c p weight` lines (reduced ids, exact
                     rationals), so the file states the problem it belongs to.
                     No `c t` line under --mode compile, which is not a track.
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

/// The command line, split into where the files go, what else gets written
/// there, and how the run is configured. Everything in the last part is a
/// [`RunConfig`] or [`SelectionCtx`] field — those flags do not carry defaults of
/// their own, they edit the two configs whose production settings the tool
/// starts from. `dot` is not among them: it shapes the OUTPUT, not the
/// preprocessing or the construction.
///
/// This is the program the `VITRI_*` research knobs are for, so it starts from
/// the env-filled configs rather than from `Default`; a caller EMBEDDING the
/// library gets `Default` and is never reconfigured behind its back by the
/// shell that launched it.
struct Args {
    input: PathBuf,
    out_dir: PathBuf,
    dot: bool,
    config: RunConfig,
    /// The construction knobs, already filled from the environment. Selection
    /// mode is decided later, from the reduced instance's own show set, on top
    /// of this value.
    selection: SelectionCtx,
}

/// The command line as typed, before anything outside it is consulted: which
/// flags appeared, and with what values.
///
/// `None` means the flag was not given, so whatever the environment-filled
/// configuration already holds stands. Keeping the two apart is what lets
/// `--help` answer while a `VITRI_*` variable in the caller's shell is
/// malformed: the loop that fills this reads nothing but `argv`.
#[derive(Default)]
struct Options {
    input: Option<PathBuf>,
    out_dir: Option<PathBuf>,
    dot: bool,
    mode: Option<Mode>,
    vtree_spec: Option<String>,
    budget_ms: Option<u64>,
    components: Option<ComponentPolicy>,
    candidates: Option<usize>,
    no_arjun: bool,
    no_simplify: bool,
}

/// The argument grammar, as a function of the argument vector. `--help` is the
/// one argument whose whole effect is to print and exit, so it does that here,
/// out of a loop that has read no environment variable yet; everything else
/// comes back as a value, an error included.
///
/// The environment is read once the whole command line is in hand, and the
/// flags are applied on top of it. A `VITRI_*` variable is therefore reported
/// after — not instead of — a bad argument on the same line.
fn parse_argv(argv: &[String]) -> Result<Args, VitriError> {
    let mut opts = Options::default();

    let next = |i: &mut usize, flag: &str| -> Result<String, VitriError> {
        *i += 1;
        argv.get(*i)
            .cloned()
            .ok_or_else(|| VitriError::config(format!("{flag} needs a value")))
    };

    let mut i = 1;
    while i < argv.len() {
        let arg = argv[i].as_str();
        // The inventory decides what a spelling means; everything left over is
        // either the input CNF or a typo. A message about an option names it by
        // its long spelling, whichever one was typed.
        let Some(opt) = OPTIONS.iter().find(|o| o.matches(arg)) else {
            if arg.starts_with('-') {
                return Err(VitriError::config(format!("unknown option {arg:?}")));
            }
            if opts.input.replace(PathBuf::from(arg)).is_some() {
                return Err(VitriError::config("expected exactly one input CNF"));
            }
            i += 1;
            continue;
        };
        match opt.key {
            OptKey::Help => {
                print!("{}", help());
                exit(0);
            }
            OptKey::OutDir => opts.out_dir = Some(PathBuf::from(next(&mut i, opt.long)?)),
            OptKey::Mode => {
                let v = next(&mut i, opt.long)?;
                opts.mode = Some(Mode::parse_mode(&v).ok_or_else(|| {
                    VitriError::config(format!(
                        "{} expects {}, got {v:?}",
                        opt.long,
                        one_of(Mode::names()),
                    ))
                })?);
            }
            OptKey::Vtree => opts.vtree_spec = Some(next(&mut i, opt.long)?),
            OptKey::BudgetMs => {
                let v = next(&mut i, opt.long)?;
                opts.budget_ms = Some(v.parse().map_err(|_| {
                    VitriError::config(format!("{} expects an integer, got {v:?}", opt.long))
                })?);
            }
            OptKey::Components => {
                let v = next(&mut i, opt.long)?;
                opts.components = Some(ComponentPolicy::parse(&v).ok_or_else(|| {
                    VitriError::config(format!(
                        "{} expects {}, got {v:?}",
                        opt.long,
                        one_of(ComponentPolicy::names()),
                    ))
                })?);
            }
            OptKey::Candidates => {
                let v = next(&mut i, opt.long)?;
                opts.candidates = Some(v.parse().map_err(|_| {
                    VitriError::config(format!(
                        "{} expects a positive integer, got {v:?}",
                        opt.long,
                    ))
                })?);
            }
            // No inert-combination guard: every mode emits at least one vtree
            // when there is anything to build one over, so `--dot` always means
            // something.
            OptKey::Dot => opts.dot = true,
            // Whether the stage these turn off is one the run's mode even has
            // cannot be settled here — the mode may still be coming from the
            // instance's own headers — so the flag only records the request, and
            // `RunConfig` judges it against the mode that ends up running.
            OptKey::NoArjun => opts.no_arjun = true,
            OptKey::NoSimplify => opts.no_simplify = true,
        }
        i += 1;
    }

    // The whole command line is in hand, so the environment can be read: each
    // flag above EDITS the two env-filled configs, and a flag that was not
    // given leaves the variable's value — or the production default — in place.
    let mut config = RunConfig::from_env_defaults()?;
    let selection = SelectionCtx::plain().with_env_defaults()?;
    if opts.mode.is_some() {
        config.mode = opts.mode;
    }
    if let Some(spec) = opts.vtree_spec {
        config.vtree_spec = spec;
    }
    if opts.budget_ms.is_some() {
        config.budget_ms = opts.budget_ms;
    }
    if let Some(policy) = opts.components {
        config.components = policy;
    }
    if let Some(n) = opts.candidates {
        config.candidates = n;
    }
    if opts.no_arjun {
        config.stages.arjun = false;
    }
    if opts.no_simplify {
        config.stages.simplify = false;
    }

    // The inert/out-of-range combinations are the config's own to judge —
    // `--candidates` above a construction that builds one vtree, or above the
    // retention ceiling — so they are checked by the ONE validator a library
    // consumer also gets, not by a second set of rules spelled here.
    config.validate()?;

    Ok(Args {
        input: opts
            .input
            .ok_or_else(|| VitriError::config("no input CNF given"))?,
        out_dir: opts
            .out_dir
            .ok_or_else(|| VitriError::config("--out-dir is required"))?,
        dot: opts.dot,
        config,
        selection,
    })
}

/// The one place a failure becomes a process exit. Everything below `run` —
/// argument parsing, the library, this binary's own file writes — reports by
/// returning a [`VitriError`], so there is a single message-and-status rule
/// instead of one per call site.
fn main() {
    // This is the crate's own binary, not a library consumer — opt in to the
    // library's diagnostic chatter (`vitri::diagnostics`, default quiet) so all
    // existing binary output is preserved.
    vitri::diagnostics::set_verbose(true);

    if let Err(e) = run() {
        eprintln!("error: {e}");
        if let Some(hint) = where_to_look(&e) {
            eprintln!("{hint}");
        }
        exit(exit_status(&e));
    }
}

/// 2 when the invocation itself is wrong — a bad argument, a flag combination
/// that would do nothing, a `VITRI_*` variable set to a value this crate cannot
/// use — and nothing ran. 1 when the invocation was fine and the work failed.
fn exit_status(e: &VitriError) -> i32 {
    match e {
        VitriError::Config { .. } | VitriError::Spec { .. } | VitriError::Env { .. } => 2,
        // A variant this binary does not know is reported as work that failed:
        // an error class it cannot recognize is not one it can blame on the
        // command line.
        _ => 1,
    }
}

/// The one-line pointer printed under the message, or `None` when the message
/// stands on its own.
///
/// Each error class is sent where its answer actually is. `--help` documents the
/// command line, so it answers a bad argument or an inert combination; it says
/// nothing about the `VITRI_*` variables, so an environment error points at the
/// file that lists them instead. A run that started and then failed has no
/// documentation to be sent to.
fn where_to_look(e: &VitriError) -> Option<&'static str> {
    match e {
        VitriError::Config { .. } | VitriError::Spec { .. } => Some("run `vitri --help` for usage"),
        VitriError::Env { .. } => Some("the supported variables are listed in docs/env.md"),
        _ => None,
    }
}

fn run() -> Result<(), VitriError> {
    let argv: Vec<String> = std::env::args().collect();
    let args = parse_argv(&argv)?;
    let started = std::time::Instant::now();

    let file = File::open(&args.input).map_err(|e| VitriError::io(&args.input, "open", &e))?;
    let reader = BufReader::new(file);
    let (formula, meta) = CnfFormula::from_dimacs(reader)
        .map_err(|e| VitriError::input(format!("parsing {}: {e}", args.input.display())))?;

    // ── The whole pipeline: preprocess, then the vtree over what is left ─────
    let run = bundle::run(&formula, &meta, &args.config, &args.selection)?;
    let bundle = &run.preprocessed;
    let paths = run.write_to_dir(
        &args.out_dir,
        bundle::components::ComponentWriteOptions { dot: args.dot },
    )?;

    print_run_report(
        &args,
        &formula,
        bundle,
        run.built(),
        paths.vtree.as_ref().map(|v| &v.components.manifest),
    );
    print_written(&paths, &args.out_dir);
    println!("elapsed:      {} ms", started.elapsed().as_millis());
    Ok(())
}

/// The `s` a count needs: nothing for one of something, `s` for none or many.
fn plural(n: usize) -> &'static str {
    if n == 1 { "" } else { "s" }
}

/// What a successful run found: what went in, what preprocessing left, the vtree
/// over it, and the component split underneath.
///
/// `built` is `None` when preprocessing resolved the instance outright — every
/// variable forced, determined, or folded into the multiplier. `count(reduced)`
/// is then 1 by definition, `count(original)` is the lift, and there is nothing
/// to build a vtree over; the record goes out without one, which is the honest
/// answer where a degenerate vtree would not be. The rest of the report is the
/// same lines either way, with the sections that describe a vtree simply absent.
///
/// A REFUTED instance takes neither of those shapes: its bundle is a synthetic
/// contradiction with a real vtree over it, so every line below reads as an
/// ordinary small run. The verdict is in the record, and it gets a line of its
/// own — without it the report describes a two-clause formula and never says
/// the count is 0.
fn print_run_report(
    args: &Args,
    formula: &CnfFormula,
    bundle: &bundle::PreprocessBundle,
    built: Option<&VtreeBuild>,
    components: Option<&bundle::components::ComponentsManifest>,
) {
    println!(
        "input:        {} ({} vars, {} clauses, mode {})",
        args.input.display(),
        formula.num_vars,
        formula.clauses.len(),
        bundle.record.mode.token(),
    );
    if bundle.record.unsat {
        println!("unsat:        preprocessing refuted the instance; count(original) = 0");
    }
    match built {
        Some(build) => {
            println!(
                "reduced:      {} vars, {} clauses  (count(original) = count(reduced) * {})",
                bundle.reduced.num_vars,
                bundle.reduced.clauses.len(),
                bundle.record.lift(),
            );
            println!(
                "vtree:        {} ({} leaves, {} nodes)",
                args.config.vtree_spec,
                build.vtree.num_leaves(),
                build.vtree.num_nodes(),
            );
        }
        None => {
            println!(
                "reduced:      0 vars — fully resolved, count(original) = {}",
                bundle.record.lift(),
            );
            println!("vtree:        none (no variables to build one over)");
        }
    }
    if let Some(manifest) = components {
        println!(
            "components:   {} ({} free variable{})",
            manifest.components.len(),
            manifest.free_vars_reduced_dimacs.len(),
            plural(manifest.free_vars_reduced_dimacs.len()),
        );
        // The candidate set, when one was asked for. Printed per component rather than
        // summed: how many DISTINCT vtrees a component's portfolio produced is the
        // number worth seeing — it is often below `--candidates` because specs
        // converge on the same tree, and it is zero for a component small enough
        // that the portfolio never ran on it.
        if candidates::retains_set(args.config.candidates) {
            match manifest.candidate_rank_metric {
                Some(metric) => {
                    let metric = metric.as_str();
                    println!("candidates:   ranked by {metric} (ascending — lower is better)")
                }
                None => {
                    println!(
                        "candidates:   no component was big enough to build a candidate set for"
                    )
                }
            }
            for (index, c) in manifest.components.iter().enumerate() {
                let n = c.vtree_candidates.len();
                let detail = if n == 0 {
                    "no candidate set — built directly, one candidate".to_string()
                } else {
                    format!(
                        "{n} distinct vtree{}: {}",
                        plural(n),
                        c.vtree_candidates
                            .iter()
                            .enumerate()
                            .map(|(rank, e)| format!("#{rank} {}", e.built_by.join("=")))
                            .collect::<Vec<_>>()
                            .join(", "),
                    )
                };
                println!("              component {index:03}: {detail}");
            }
        }
    }
}

/// Every file the run wrote, one per line under a single `wrote:` label.
///
/// The two directories a component split fills are named with a count rather
/// than listed: one line for a hundred component files is what a reader can use,
/// and the manifest above them is what names each.
fn print_written(paths: &bundle::RunPaths, out_dir: &std::path::Path) {
    let dir_line = |dir: &str, n: usize| {
        if n > 0 {
            println!("              {}/ ({n} files)", out_dir.join(dir).display());
        }
    };
    println!("wrote:        {}", paths.bundle.reduced_cnf.display());
    println!("              {}", paths.bundle.record.display());
    if let Some(vtree) = &paths.vtree {
        for p in std::iter::once(&vtree.vtree).chain(vtree.dot.iter()) {
            println!("              {}", p.display());
        }
        let comp_paths = &vtree.components.paths;
        println!("              {}", comp_paths.manifest.display());
        dir_line(bundle::components::COMPONENTS_DIR, comp_paths.files.len());
        dir_line(
            bundle::components::CANDIDATES_DIR,
            comp_paths.candidates.len(),
        );
    }
}
