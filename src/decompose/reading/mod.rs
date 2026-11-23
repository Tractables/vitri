//! Readings of a cut from a sample of models.
//!
//! Split the variables into an inside set X and the rest Y. Models i and j of
//! the sample are compatible at that cut when the X half of i joined with the
//! Y half of j is again a model; only the clauses with literals on both sides
//! can fail. The reading of the cut is
//!
//! ```text
//! max(log2 max(min(R, N), 1), x)
//! ```
//!
//! with R and N the number of distinct rows and columns of the k-by-k
//! compatibility matrix (Chao1-extrapolated from the classes holding one and
//! two models, unless the sample is exhaustive) and x the surprisal of its
//! off-diagonal density. The interval matrix of a leaf order holds the reading,
//! clipped at `2 log2 k - 3`, of every run of consecutive blocks of the order;
//! the interval dynamic programs build trees from it.
//!
//! The work is bit sets over the models. For a crossing clause c, `open_in(c)`
//! is the set of models whose X literals of c are all false and `open_out(c)`
//! the set whose Y literals are. Model i is incompatible with model j exactly
//! when some c has i in `open_in(c)` and j in `open_out(c)`, so the row of i is
//! the union of `open_out(c)` over the clauses with i in `open_in(c)`. Models
//! that leave the same clauses open have the same row, so a row is formed once
//! per distinct set of open clauses.

use rustc_hash::FxHashMap;

#[cfg(test)]
mod tests;

/// A sample of models of one CNF, one bit set over the models per literal.
pub(crate) struct ModelSample {
    k: usize,
    words: usize,
    /// Set when the sample is taken to be every model, so classes are counted
    /// rather than extrapolated.
    exact: bool,
    /// For literal `2v + positive`, the models where that literal is false.
    falses: Vec<u64>,
}

/// The CNF over `n` variables as flat occurrence lists.
pub(crate) struct ClauseList {
    var: Vec<u32>,
    positive: Vec<bool>,
    start: Vec<usize>,
}

impl ClauseList {
    /// Clauses as DIMACS literals over variables `1..=n`.
    pub(crate) fn new(clauses: &[Vec<i32>]) -> Self {
        let mut var = Vec::new();
        let mut positive = Vec::new();
        let mut start = vec![0];
        for clause in clauses {
            for &lit in clause {
                var.push(lit.unsigned_abs() - 1);
                positive.push(lit > 0);
            }
            start.push(var.len());
        }
        Self {
            var,
            positive,
            start,
        }
    }

    pub(crate) fn len(&self) -> usize {
        self.start.len() - 1
    }

    /// The number of literal occurrences.
    pub(crate) fn occurrences(&self) -> usize {
        self.var.len()
    }

    /// The variables of clause `c`, from 0.
    pub(crate) fn vars(&self, c: usize) -> &[u32] {
        &self.var[self.start[c]..self.start[c + 1]]
    }

    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub(crate) fn satisfied_by(&self, model: &[bool]) -> bool {
        (0..self.len()).all(|c| {
            (self.start[c]..self.start[c + 1])
                .any(|o| model[self.var[o] as usize] == self.positive[o])
        })
    }
}

impl ModelSample {
    /// A draw of assignments over `n` variables. A draw with fewer distinct
    /// assignments than half its size is replaced by its distinct ones; then
    /// assignments that are not models are dropped, and if fewer than half of
    /// the rest are distinct, the distinct ones are kept and the sample is
    /// exact.
    pub(crate) fn new(n: usize, clauses: &ClauseList, draw: Vec<Vec<bool>>) -> Self {
        let mut x = draw;
        let distinct = distinct_rows(&x);
        if distinct.len() * 2 < x.len() {
            x = distinct;
        }
        x.retain(|m| clauses.satisfied_by(m));
        let distinct = distinct_rows(&x);
        let exact = !x.is_empty() && distinct.len() * 2 < x.len();
        if exact {
            x = distinct;
        }
        let k = x.len();
        let words = k.div_ceil(64).max(1);
        let mut falses = vec![0u64; 2 * n * words];
        for (i, model) in x.iter().enumerate() {
            let (w, bit) = (i / 64, 1u64 << (i % 64));
            for (v, &value) in model.iter().enumerate() {
                // Literal 2v + 1 (positive) is false where v is false.
                falses[(2 * v + usize::from(!value)) * words + w] |= bit;
            }
        }
        Self {
            k,
            words,
            exact,
            falses,
        }
    }

    /// Words in a bit set over the models.
    pub(crate) fn words(&self) -> usize {
        self.words
    }

    /// The clip on a reading: `2 log2 k - 3`.
    pub(crate) fn cap(&self) -> f64 {
        2.0 * (self.k.max(2) as f64).log2() - 3.0
    }

    fn falses(&self, var: u32, positive: bool) -> &[u64] {
        let l = 2 * var as usize + usize::from(positive);
        &self.falses[l * self.words..(l + 1) * self.words]
    }

    fn full_mask(&self) -> impl Iterator<Item = u64> + '_ {
        (0..self.words).map(|w| {
            let bits = (self.k - (w * 64).min(self.k)).min(64);
            if bits == 64 { !0 } else { (1u64 << bits) - 1 }
        })
    }
}

fn distinct_rows(x: &[Vec<bool>]) -> Vec<Vec<bool>> {
    let mut seen = rustc_hash::FxHashSet::default();
    x.iter()
        .filter(|m| seen.insert(m.as_slice()))
        .cloned()
        .collect()
}

/// Class counts of the rows of one side of the compatibility matrix.
struct Classes {
    count: usize,
    singletons: usize,
    doubletons: usize,
}

/// Reads cuts of one CNF against one sample, reusing its buffers.
pub(crate) struct Reader<'a> {
    clauses: &'a ClauseList,
    sample: &'a ModelSample,
    /// Per crossing clause that some model leaves open on each side, the
    /// models open inside and the models open outside.
    open_in: Vec<u64>,
    open_out: Vec<u64>,
    /// The same with clauses of equal outside sets merged.
    merged_in: Vec<u64>,
    merged_out: Vec<u64>,
    /// Per model, its membership in the merged clauses, 64 to a word: bit b
    /// of word w is merged clause `64 w + b`.
    member: Vec<u64>,
    /// Words in a model's membership.
    member_words: usize,
    /// Models grouped by membership: the group of each model, one model and
    /// the size of each group.
    rep_of: Vec<u32>,
    rep_model: Vec<u32>,
    weight: Vec<u32>,
    /// The incompatible models of each group, then of each model by column.
    rows: Vec<u64>,
    cols: Vec<u64>,
    table: Vec<u64>,
    order: Vec<(u64, u32)>,
    /// The models' bit sets with every model set.
    mask: Vec<u64>,
    pub(crate) reads: u64,
}

impl<'a> Reader<'a> {
    pub(crate) fn new(clauses: &'a ClauseList, sample: &'a ModelSample) -> Self {
        Self {
            clauses,
            sample,
            open_in: Vec::new(),
            open_out: Vec::new(),
            merged_in: Vec::new(),
            merged_out: Vec::new(),
            member: Vec::new(),
            member_words: 0,
            rep_of: Vec::new(),
            rep_model: Vec::new(),
            weight: Vec::new(),
            rows: Vec::new(),
            cols: Vec::new(),
            table: vec![0; 256 * sample.words],
            order: Vec::new(),
            mask: sample.full_mask().collect(),
            reads: 0,
        }
    }

    /// The unclipped reading of the cut with `inside` the inside variables.
    pub(crate) fn raw(&mut self, inside: &[bool]) -> f64 {
        self.reads += 1;
        let (k, words) = (self.sample.k, self.sample.words);
        if k == 0 || self.clauses.is_empty() || !self.open_sets(inside) {
            return 0.0;
        }
        self.merge_by_outside();
        self.group_models();
        self.form_rows();
        let rows = classes(&self.rows, words, Some(&self.weight), &mut self.order);
        let (mut total, mut diag) = (0.0, 0.0);
        for (row, &w) in self.rows.chunks_exact(words).zip(&self.weight) {
            let ones: u32 = row.iter().map(|w| w.count_ones()).sum();
            total += f64::from(w) * (k - ones as usize) as f64;
        }
        for (i, &r) in self.rep_of.iter().enumerate() {
            if (self.rows[r as usize * words + i / 64] >> (i % 64)) & 1 == 0 {
                diag += 1.0;
            }
        }
        self.transpose_rows();
        let cols = classes(&self.cols, words, None, &mut self.order);
        let estimate = |c: &Classes| {
            if self.sample.exact {
                c.count as f64
            } else {
                chao1(c.count, c.singletons, c.doubletons)
            }
        };
        let (r_hat, n_hat) = (estimate(&rows), estimate(&cols));
        let offmean = if k >= 2 {
            (total - diag) / (k as f64 * (k - 1) as f64)
        } else {
            1.0
        };
        let x_hat = if offmean > 0.0 {
            -offmean.log2()
        } else {
            2.0 * (k.max(2) as f64).log2()
        };
        r_hat.min(n_hat).max(1.0).log2().max(x_hat)
    }

    /// The open sets of the crossing clauses that some model leaves open on
    /// each side. False when no clause crosses the cut.
    fn open_sets(&mut self, inside: &[bool]) -> bool {
        self.open_in.clear();
        self.open_out.clear();
        let mut crossing = false;
        let cl = self.clauses;
        for span in cl.start.windows(2) {
            let occ = span[0]..span[1];
            let n_in = occ.clone().filter(|&o| inside[cl.var[o] as usize]).count();
            if n_in == 0 || n_in == occ.len() {
                continue;
            }
            crossing = true;
            let base = self.open_in.len();
            self.open_in.extend_from_slice(&self.mask);
            self.open_out.extend_from_slice(&self.mask);
            for o in occ {
                let f = self.sample.falses(cl.var[o], cl.positive[o]);
                let dst = if inside[cl.var[o] as usize] {
                    &mut self.open_in[base..]
                } else {
                    &mut self.open_out[base..]
                };
                for (d, s) in dst.iter_mut().zip(f) {
                    *d &= s;
                }
            }
            let nonempty = |v: &[u64]| v.iter().any(|&w| w != 0);
            if !nonempty(&self.open_in[base..]) || !nonempty(&self.open_out[base..]) {
                self.open_in.truncate(base);
                self.open_out.truncate(base);
            }
        }
        crossing
    }

    /// Clauses with the same outside set act as one clause open inside for
    /// the union of their inside sets.
    fn merge_by_outside(&mut self) {
        let words = self.sample.words;
        let c = self.open_out.len() / words;
        self.order.clear();
        for (j, set) in self.open_out.chunks_exact(words).enumerate() {
            self.order.push((row_hash(set), j as u32));
        }
        self.order.sort_unstable();
        self.merged_in.clear();
        self.merged_out.clear();
        let mut a = 0;
        while a < c {
            let mut b = a;
            while b < c && self.order[b].0 == self.order[a].0 {
                b += 1;
            }
            let first = self.merged_out.len() / words;
            for &(_, j) in &self.order[a..b] {
                let j = j as usize;
                let out = &self.open_out[j * words..(j + 1) * words];
                let into = (first..self.merged_out.len() / words)
                    .find(|&m| &self.merged_out[m * words..(m + 1) * words] == out);
                match into {
                    Some(m) => {
                        let src = &self.open_in[j * words..(j + 1) * words];
                        for (d, s) in self.merged_in[m * words..(m + 1) * words]
                            .iter_mut()
                            .zip(src)
                        {
                            *d |= s;
                        }
                    }
                    None => {
                        self.merged_out.extend_from_slice(out);
                        self.merged_in
                            .extend_from_slice(&self.open_in[j * words..(j + 1) * words]);
                    }
                }
            }
            a = b;
        }
    }

    /// Groups the models by the merged clauses they leave open inside: models
    /// of one group have the same row. The memberships are the transpose of
    /// the clause-by-model matrix of `merged_in`, in 64-by-64 blocks.
    fn group_models(&mut self) {
        let (k, words) = (self.sample.k, self.sample.words);
        let clauses = self.merged_in.len() / words;
        let mw = clauses.div_ceil(64);
        self.member_words = mw;
        self.member.clear();
        self.member.resize(k * mw, 0);
        let mut block = [0u64; 64];
        for bc in 0..mw {
            for bm in 0..words {
                for (t, slot) in block.iter_mut().enumerate() {
                    let c = 64 * bc + t;
                    *slot = if c < clauses {
                        self.merged_in[c * words + bm]
                    } else {
                        0
                    };
                }
                transpose64(&mut block);
                for (t, &w) in block.iter().enumerate() {
                    let i = 64 * bm + t;
                    if i < k {
                        self.member[i * mw + bc] = w;
                    }
                }
            }
        }
        self.rep_of.clear();
        self.rep_model.clear();
        self.weight.clear();
        let mut first: FxHashMap<&[u64], u32> = FxHashMap::default();
        for i in 0..k {
            let key = &self.member[i * mw..(i + 1) * mw];
            let next = first.len() as u32;
            let r = *first.entry(key).or_insert(next);
            if r == next {
                self.rep_model.push(i as u32);
                self.weight.push(0);
            }
            self.weight[r as usize] += 1;
            self.rep_of.push(r);
        }
    }

    /// The row of each group: the union of the outside sets of its merged
    /// clauses, eight clauses at a time through a table of their unions. The
    /// table holds the unions for the bytes some group has and, to build
    /// them, the bytes those have with their low bits cleared.
    fn form_rows(&mut self) {
        let words = self.sample.words;
        let clauses = self.merged_out.len() / words;
        let mw = self.member_words;
        let byte = |member: &[u64], m: u32, g: usize| {
            (member[m as usize * mw + g / 8] >> (8 * (g % 8))) as u8 as usize
        };
        let reps = self.rep_model.len();
        self.rows.clear();
        self.rows.resize(reps * words, 0);
        let mut needed = [false; 256];
        for g in 0..clauses.div_ceil(8) {
            needed.fill(false);
            for &m in &self.rep_model {
                needed[byte(&self.member, m, g)] = true;
            }
            for b in (1..256).rev() {
                if needed[b] {
                    needed[b & (b - 1)] = true;
                }
            }
            for b in (1..256).filter(|&b| needed[b]) {
                let low = 8 * g + b.trailing_zeros() as usize;
                let prev = b & (b - 1);
                let (lo, hi) = self.table.split_at_mut(b * words);
                let src = &self.merged_out[low * words..(low + 1) * words];
                for ((d, p), s) in hi[..words].iter_mut().zip(&lo[prev * words..]).zip(src) {
                    *d = p | s;
                }
            }
            for (r, &m) in self.rep_model.iter().enumerate() {
                let b = byte(&self.member, m, g);
                if b != 0 {
                    let src = &self.table[b * words..(b + 1) * words];
                    for (d, s) in self.rows[r * words..(r + 1) * words].iter_mut().zip(src) {
                        *d |= s;
                    }
                }
            }
        }
    }

    /// The incompatible models of each model as a column: the transpose of
    /// the model-by-model row matrix, in 64-by-64 blocks.
    fn transpose_rows(&mut self) {
        let (k, words) = (self.sample.k, self.sample.words);
        self.cols.clear();
        self.cols.resize(k * words, 0);
        let mut block = [0u64; 64];
        for bi in 0..words {
            for bj in 0..words {
                for (t, slot) in block.iter_mut().enumerate() {
                    let i = 64 * bi + t;
                    *slot = if i < k {
                        self.rows[self.rep_of[i] as usize * words + bj]
                    } else {
                        0
                    };
                }
                transpose64(&mut block);
                for (t, &w) in block.iter().enumerate() {
                    let j = 64 * bj + t;
                    if j < k {
                        self.cols[j * words + bi] = w;
                    }
                }
            }
        }
    }
}

/// Distinct rows of `rows` (`words` each) weighted by `weight` (1 each when
/// none): sort by a hash of the row and compare the rows within a run of
/// equal hashes.
fn classes(
    rows: &[u64],
    words: usize,
    weight: Option<&[u32]>,
    order: &mut Vec<(u64, u32)>,
) -> Classes {
    order.clear();
    for (r, row) in rows.chunks_exact(words).enumerate() {
        order.push((row_hash(row), r as u32));
    }
    order.sort_unstable();
    let row = |r: u32| &rows[r as usize * words..(r as usize + 1) * words];
    let mut classes = Classes {
        count: 0,
        singletons: 0,
        doubletons: 0,
    };
    let mut run: Vec<(u32, u32)> = Vec::new();
    let mut a = 0;
    while a < order.len() {
        let mut b = a;
        while b < order.len() && order[b].0 == order[a].0 {
            b += 1;
        }
        run.clear();
        for &(_, r) in &order[a..b] {
            let w = weight.map_or(1, |w| w[r as usize]);
            match run.iter_mut().find(|(q, _)| row(*q) == row(r)) {
                Some((_, acc)) => *acc += w,
                None => run.push((r, w)),
            }
        }
        for &(_, w) in &run {
            classes.count += 1;
            classes.singletons += usize::from(w == 1);
            classes.doubletons += usize::from(w == 2);
        }
        a = b;
    }
    classes
}

/// Transposes a 64-by-64 bit matrix held as 64 rows, bit b of row t being
/// entry (t, b).
fn transpose64(a: &mut [u64; 64]) {
    let mut j = 32;
    let mut m: u64 = 0x0000_0000_ffff_ffff;
    while j != 0 {
        let mut k = 0;
        while k < 64 {
            let t = ((a[k] >> j) ^ a[k + j]) & m;
            a[k] ^= t << j;
            a[k + j] ^= t;
            k = (k + j + 1) & !j;
        }
        j >>= 1;
        m ^= m << j;
    }
}

/// The readings of one sample by variable set. A set and its complement have
/// the same reading (the compatibility matrix is transposed), so the pair is
/// read once.
pub(crate) struct Readings<'a> {
    reader: Reader<'a>,
    /// Readings by the bits of the set or its complement, whichever leaves out
    /// variable 0.
    known: FxHashMap<Box<[u64]>, f64>,
    key: Vec<u64>,
}

impl<'a> Readings<'a> {
    pub(crate) fn new(clauses: &'a ClauseList, sample: &'a ModelSample) -> Self {
        Self {
            reader: Reader::new(clauses, sample),
            known: FxHashMap::default(),
            key: Vec::new(),
        }
    }

    pub(crate) fn sample(&self) -> &'a ModelSample {
        self.reader.sample
    }

    /// The sets read so far, each counted once with its complement.
    pub(crate) fn reads(&self) -> u64 {
        self.reader.reads
    }

    /// The unclipped reading of the cut with `inside` the inside variables,
    /// read unless it is known; none when it is not known and `go_on`, asked
    /// before reading, says to stop.
    pub(crate) fn get(&mut self, inside: &[bool], go_on: impl FnOnce() -> bool) -> Option<f64> {
        let flip = inside.first().copied().unwrap_or(false);
        self.key.clear();
        self.key.resize(inside.len().div_ceil(64), 0);
        for (i, &x) in inside.iter().enumerate() {
            if x != flip {
                self.key[i / 64] |= 1 << (i % 64);
            }
        }
        if let Some(&r) = self.known.get(self.key.as_slice()) {
            return Some(r);
        }
        if !go_on() {
            return None;
        }
        let r = self.reader.raw(inside);
        self.known.insert(self.key.as_slice().into(), r);
        Some(r)
    }

    /// [`Readings::get`], reading whatever is not known.
    pub(crate) fn read(&mut self, inside: &[bool]) -> f64 {
        self.get(inside, || true).unwrap_or_default()
    }
}

/// The interval matrix of a leaf order over `n` variables: blocks of
/// `max(4, n / 64)` consecutive variables (a last block of one variable joins
/// the one before), and entry `(s, t)`, `s <= t`, the clipped reading of blocks
/// `s..=t` against the rest. The whole order's entry is left at 0, as are the
/// entries below the diagonal. Returns the block bounds and the matrix by rows,
/// or none when `go_on`, asked before each reading not yet in `readings`, says
/// to stop.
pub(crate) fn interval_matrix(
    readings: &mut Readings<'_>,
    order: &[usize],
    mut go_on: impl FnMut() -> bool,
) -> Option<(Vec<usize>, Vec<f64>)> {
    let n = order.len();
    let bounds = block_bounds(n);
    let g = bounds.len() - 1;
    let cap = readings.sample().cap();
    let mut matrix = vec![0.0; g * g];
    let mut inside = vec![false; n];
    for s in 0..g {
        inside.iter_mut().for_each(|b| *b = false);
        for t in s..g {
            for &v in &order[bounds[t]..bounds[t + 1]] {
                inside[v] = true;
            }
            if !(s == 0 && t == g - 1) {
                matrix[s * g + t] = readings.get(&inside, &mut go_on)?.min(cap);
            }
        }
    }
    Some((bounds, matrix))
}

pub(crate) fn block_bounds(n: usize) -> Vec<usize> {
    let g = (n / 64).max(4);
    let mut bounds: Vec<usize> = (0..n).step_by(g).collect();
    bounds.push(n);
    let m = bounds.len();
    if m > 2 && bounds[m - 1] - bounds[m - 2] < 2 {
        bounds.remove(m - 2);
    }
    bounds
}

fn chao1(classes: usize, f1: usize, f2: usize) -> f64 {
    let f1 = f1 as f64;
    classes as f64 + f1 * (f1 - 1.0) / (2.0 * (f2 as f64 + 1.0))
}

fn row_hash(row: &[u64]) -> u64 {
    let mut h = 0x243f_6a88_85a3_08d3u64;
    for &w in row {
        h = (h ^ w).wrapping_mul(0x9e37_79b9_7f4a_7c15).rotate_left(29);
    }
    h
}
