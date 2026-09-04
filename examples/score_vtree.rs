//! Score (CNF, vtree) pairs with exactly what candidate selection ranks on.
//!
//! An offline measuring instrument for fits; it ships with no release.
//!
//! Input: a TSV on argv[1] (or stdin) with three columns, `id`, `cnf_path`,
//! `vtree_path`. Rows sharing a CNF should be adjacent; the formula is parsed
//! once per run of equal paths.
//!
//! Output: one JSON object per line on stdout with the five `VtreeScores`
//! fields, the eleven weighted cost terms, the two file paths and a SHA-256 of
//! each. The terms are asserted to sum to the cost the selector would use.

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
    let mut cached: Option<(String, CnfFormula, String)> = None;
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
        if cached.as_ref().map(|(p, _, _)| p.as_str()) != Some(cnf_path) {
            match read_formula(cnf_path) {
                Ok((formula, sha)) => cached = Some((cnf_path.to_string(), formula, sha)),
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
        let (_, formula, cnf_sha) = cached.as_ref().unwrap();

        let vtree_bytes = match std::fs::read(vtree_path) {
            Ok(b) => b,
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
        let vtree_sha = sha256_hex(&vtree_bytes);
        let vtree_text = String::from_utf8_lossy(&vtree_bytes).into_owned();
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
             \"cnf\":{cp},\"cnf_sha256\":{cs},\"vtree\":{vp},\"vtree_sha256\":{vs}}}",
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
            cs = json_str(cnf_sha),
            vp = json_str(vtree_path),
            vs = json_str(&vtree_sha),
        )
        .unwrap();
        n_ok += 1;
    }
    out.flush().unwrap();
    eprintln!("scored {n_ok}, errors {n_err}");
}

fn read_formula(path: &str) -> Result<(CnfFormula, String), String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    let sha = sha256_hex(&bytes);
    let reader = BufReader::new(std::io::Cursor::new(bytes));
    let (formula, _meta) = CnfFormula::from_dimacs(reader).map_err(|e| e.to_string())?;
    Ok((formula, sha))
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

// ---- SHA-256, so a row carries the provenance of the bytes it scored.

const K: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

fn sha256_hex(data: &[u8]) -> String {
    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];
    let bitlen = (data.len() as u64).wrapping_mul(8);
    let mut msg = data.to_vec();
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bitlen.to_be_bytes());
    for chunk in msg.chunks_exact(64) {
        let mut w = [0u32; 64];
        for (i, word) in chunk.chunks_exact(4).enumerate() {
            w[i] = u32::from_be_bytes([word[0], word[1], word[2], word[3]]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let (mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh) =
            (h[0], h[1], h[2], h[3], h[4], h[5], h[6], h[7]);
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let t1 = hh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        h[0] = h[0].wrapping_add(a);
        h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c);
        h[3] = h[3].wrapping_add(d);
        h[4] = h[4].wrapping_add(e);
        h[5] = h[5].wrapping_add(f);
        h[6] = h[6].wrapping_add(g);
        h[7] = h[7].wrapping_add(hh);
    }
    h.iter().map(|x| format!("{x:08x}")).collect()
}
