//! Score (CNF, vtree) pairs with exactly what candidate selection ranks on.
//!
//! An offline measuring instrument for fits; it ships with no release.
//!
//! Input: a TSV on argv[1] (or stdin) with three columns, `id`, `cnf_path`,
//! `vtree_path`. Rows sharing a CNF should be adjacent; the formula is parsed
//! once per run of equal paths.
//!
//! Output: one JSON object per line on stdout with the five `VtreeScores`
//! fields, the eleven weighted cost terms and the two file paths. The terms
//! are asserted to sum to the cost the selector would use.

use std::io::{BufReader, Read, Write};

use vitri::cnf::CnfFormula;
use vitri::score::{VtreeScores, vtree_cost_terms};
use vitri::vtree::Vtree;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let text = if args.len() > 1 {
        std::fs::read_to_string(&args[1]).expect("read job list")
    } else {
        let mut s = String::new();
        std::io::stdin().read_to_string(&mut s).expect("read stdin");
        s
    };
    let out = std::io::stdout();
    let mut out = std::io::BufWriter::new(out.lock());
    let mut cached: Option<(String, CnfFormula)> = None;
    let mut n_ok = 0usize;
    let mut n_err = 0usize;

    for line in text.lines() {
        let line = line.trim_end_matches(['\r', '\n']);
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut f = line.split('\t');
        let (id, cnf_path, vtree_path) = match (f.next(), f.next(), f.next()) {
            (Some(a), Some(b), Some(c)) => (a, b, c),
            _ => {
                eprintln!("bad job line: {line}");
                n_err += 1;
                continue;
            }
        };
        if cached.as_ref().map(|(p, _)| p.as_str()) != Some(cnf_path) {
            match read_formula(cnf_path) {
                Ok(formula) => cached = Some((cnf_path.to_string(), formula)),
                Err(e) => {
                    writeln!(
                        out,
                        "{{\"id\":{},\"error\":{}}}",
                        json_str(id),
                        json_str(&format!("cnf: {e}"))
                    )
                    .unwrap();
                    n_err += 1;
                    cached = None;
                    continue;
                }
            }
        }
        let (_, formula) = cached.as_ref().unwrap();

        let vtree_text = match std::fs::read_to_string(vtree_path) {
            Ok(t) => t,
            Err(e) => {
                writeln!(
                    out,
                    "{{\"id\":{},\"error\":{}}}",
                    json_str(id),
                    json_str(&format!("vtree read: {e}"))
                )
                .unwrap();
                n_err += 1;
                continue;
            }
        };
        let vtree = match Vtree::from_vtree_text(&vtree_text) {
            Ok(v) => v,
            Err(e) => {
                writeln!(
                    out,
                    "{{\"id\":{},\"error\":{}}}",
                    json_str(id),
                    json_str(&format!("vtree parse: {e}"))
                )
                .unwrap();
                n_err += 1;
                continue;
            }
        };
        let scores = match VtreeScores::compute(&vtree, formula, None) {
            Ok(s) => s,
            Err(e) => {
                writeln!(
                    out,
                    "{{\"id\":{},\"error\":{}}}",
                    json_str(id),
                    json_str(&format!("compute: {e}"))
                )
                .unwrap();
                n_err += 1;
                continue;
            }
        };
        let terms = vtree_cost_terms(&vtree, formula).expect("terms after compute");
        let total: f64 = terms.iter().sum();
        assert!(
            (total - scores.cost).abs() <= 1e-9 * (1.0 + scores.cost.abs()),
            "term sum {total} != cost {} for {vtree_path}",
            scores.cost
        );

        writeln!(
            out,
            "{{\"id\":{id},\"cost\":{cost:.17},\"term_sum\":{total:.17},\
             \"peak_context_width_all\":{peak},\"max_clause_load\":{mcl},\
             \"clause_load_stddev\":{sd:.17},\
             \"tight\":{t0:.17},\"excess_half\":{t1:.17},\"clause_load_bits\":{t2:.17},\
             \"high_load_25\":{t3:.17},\"chain_3_40\":{t4:.17},\"join_neg_half\":{t5:.17},\
             \"directional_half\":{t6:.17},\"output_gap_16\":{t7:.17},\
             \"extreme_chain_4\":{t8:.17},\"extreme_join_32\":{t9:.17},\
             \"successor_guard\":{t10:.17},\"zero_tight\":{zt},\
             \"num_vars\":{nv},\"num_clauses\":{nc},\"num_leaves\":{nl},\
             \"cnf\":{cp},\"vtree\":{vp}}}",
            id = json_str(id),
            cost = scores.cost,
            total = total,
            peak = scores.peak_context_width_all,
            mcl = scores.max_clause_load,
            sd = scores.clause_load_stddev,
            t0 = terms[0],
            t1 = terms[1],
            t2 = terms[2],
            t3 = terms[3],
            t4 = terms[4],
            t5 = terms[5],
            t6 = terms[6],
            t7 = terms[7],
            t8 = terms[8],
            t9 = terms[9],
            t10 = terms[10],
            zt = terms[0] == 0.0,
            nv = formula.num_vars,
            nc = formula.clauses.len(),
            nl = vtree.num_leaves(),
            cp = json_str(cnf_path),
            vp = json_str(vtree_path),
        )
        .unwrap();
        n_ok += 1;
    }
    out.flush().unwrap();
    eprintln!("scored {n_ok}, errors {n_err}");
}

fn read_formula(path: &str) -> Result<CnfFormula, String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    let reader = BufReader::new(std::io::Cursor::new(bytes));
    let (formula, _meta) = CnfFormula::from_dimacs(reader).map_err(|e| e.to_string())?;
    Ok(formula)
}

fn json_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}
