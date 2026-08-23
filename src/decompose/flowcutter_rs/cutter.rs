//! The incremental cut search: grow a source side and a target side, augment
//! flow whenever one reaches the other, then pierce the current cut at the
//! frontier node scoring best on hop distance, and repeat.
//!
//! `BasicCutter` is one source-target pair. `MultiCutter` runs several and
//! advances whichever currently has the smallest cut, committing to a new
//! current cut only once the smaller side has grown as well — the Pareto rule
//! that makes the search anytime, so `super`'s outer loop can stop at any
//! point and still hold a usable separator.
//!
//! Everything here is in the vertex-split index space `expanded` defines: nodes
//! and arcs are the split graph's, so a cut is an edge cut that only becomes a
//! vertex separator once the caller maps it back. Piercing scores read hop
//! distances taken once at `init` and never rescored, so they stay a fixed
//! heuristic as the cut moves.

use super::*;

pub(super) const SOURCE_SIDE: usize = 0;

pub(super) const TARGET_SIDE: usize = 1;

/// Per-arc flow value, stored offset by 1 to fit in u8 for compactness.
/// Actual flow = stored - 1, so 0/1/2 ↔ flow -1/0/1.
pub(super) struct Flow {
    f: Vec<u8>,
}

impl Flow {
    fn new(arc_count: usize) -> Self {
        Flow {
            f: vec![1u8; arc_count],
        }
    }
    fn clear(&mut self) {
        for x in &mut self.f {
            *x = 1;
        }
    }
    #[inline]
    fn get(&self, a: u32) -> i8 {
        (self.f[a as usize] as i8) - 1
    }
    #[inline]
    fn increase(&mut self, a: u32, back: u32) {
        self.f[a as usize] += 1;
        self.f[back as usize] = 2 - self.f[a as usize];
    }
    #[inline]
    fn decrease(&mut self, a: u32, back: u32) {
        self.f[a as usize] -= 1;
        self.f[back as usize] = 2 - self.f[a as usize];
    }
}

pub(super) struct NodeSet {
    inside: Vec<bool>,
    count: u32,
    extra: Option<u32>,
}

impl NodeSet {
    fn new(n: u32) -> Self {
        NodeSet {
            inside: vec![false; n as usize],
            count: 0,
            extra: None,
        }
    }
    fn clear(&mut self) {
        for x in &mut self.inside {
            *x = false;
        }
        self.count = 0;
        self.extra = None;
    }
    fn set_extra(&mut self, x: u32) {
        debug_assert!(!self.inside[x as usize]);
        debug_assert!(self.extra.is_none());
        self.inside[x as usize] = true;
        self.count += 1;
        self.extra = Some(x);
    }
}

pub(super) fn bfs_hop_distance(g: &Exp<'_>, source: u32, dist: &mut [i32], queue: &mut Vec<u32>) {
    for d in dist.iter_mut() {
        *d = i32::MAX;
    }
    dist[source as usize] = 0;
    queue.clear();
    queue.push(source);
    let mut head = 0usize;
    while head < queue.len() {
        let x = queue[head];
        head += 1;
        let dx = dist[x as usize];
        g.for_out_arc(x, |xy| {
            let y = g.head(xy);
            if dist[y as usize] > dx + 1 {
                dist[y as usize] = dx + 1;
                queue.push(y);
            }
        });
    }
}

pub(super) struct Exp<'a> {
    pub(super) g: &'a OrigGraph,
    pub(super) a_orig: u32,
}

impl Exp<'_> {
    #[inline]
    fn head(&self, arc: u32) -> u32 {
        exp_head(self.g, self.a_orig, arc)
    }
    #[inline]
    fn for_out_arc<F: FnMut(u32)>(&self, x: u32, f: F) {
        exp_out_arcs(self.g, self.a_orig, x, f);
    }
}

pub(super) struct BasicCutter {
    assim: [NodeSet; 2],
    /// Arc IDs, not node IDs: arcs leaving `assim[side]` that carry flow.
    front: [Vec<u32>; 2],
    reach: [NodeSet; 2],
    /// Arc IDs indexed by node: the arc used to reach each node, for walking
    /// an augmenting path back to its source.
    predecessor: [Vec<u32>; 2],
    flow: Flow,
    tmp_dfs: Vec<u32>,
    /// Hop distances from the two original endpoints, fixed at `init` and
    /// never rescored as the cut grows.
    node_dist: [Vec<i32>; 2],
    cut_available: bool,
}

pub(super) const NO_PRED: u32 = u32::MAX;

impl BasicCutter {
    fn new(n_exp: u32, a_exp: u32) -> Self {
        BasicCutter {
            assim: [NodeSet::new(n_exp), NodeSet::new(n_exp)],
            front: [Vec::new(), Vec::new()],
            reach: [NodeSet::new(n_exp), NodeSet::new(n_exp)],
            predecessor: [vec![NO_PRED; n_exp as usize], vec![NO_PRED; n_exp as usize]],
            flow: Flow::new(a_exp as usize),
            tmp_dfs: Vec::with_capacity(n_exp as usize),
            node_dist: [
                vec![i32::MAX; n_exp as usize],
                vec![i32::MAX; n_exp as usize],
            ],
            cut_available: false,
        }
    }

    fn init(&mut self, exp: &Exp, a_orig: u32, p: (u32, u32)) {
        for s in 0..2 {
            self.assim[s].clear();
            self.reach[s].clear();
            self.front[s].clear();
            for p in self.predecessor[s].iter_mut() {
                *p = NO_PRED;
            }
        }
        self.flow.clear();

        self.assim[SOURCE_SIDE].set_extra(p.0);
        self.reach[SOURCE_SIDE].set_extra(p.0);
        self.assim[TARGET_SIDE].set_extra(p.1);
        self.reach[TARGET_SIDE].set_extra(p.1);

        let mut q = Vec::with_capacity(exp.g.n as usize * 2);
        bfs_hop_distance(exp, p.0, &mut self.node_dist[SOURCE_SIDE], &mut q);
        bfs_hop_distance(exp, p.1, &mut self.node_dist[TARGET_SIDE], &mut q);

        self.grow_reachable_sets(exp, a_orig, SOURCE_SIDE);
        self.grow_assimilated_sets(exp, a_orig);

        self.cut_available = true;
    }

    fn is_saturated(&self, exp: &Exp, a_orig: u32, direction: usize, arc: u32) -> bool {
        let arc = if direction == TARGET_SIDE {
            exp_back(exp.g, a_orig, arc)
        } else {
            arc
        };
        let cap = exp_capacity(a_orig, arc);
        let flow = self.flow.get(arc);
        cap == flow
    }

    /// Grows `reach[pierced_side]`; on hitting the opposite assimilated set it
    /// augments `flow` and continues, then conditionally regrows the other
    /// side once no augmenting path remains.
    fn grow_reachable_sets(&mut self, exp: &Exp, a_orig: u32, pierced_side: usize) {
        let my_src = pierced_side;
        let my_tgt = 1 - pierced_side;

        let mut was_flow_augmented = false;

        loop {
            let mut target_hit: i64 = -1;

            let extra = match self.reach[my_src].extra.take() {
                Some(x) => x,
                None => break,
            };

            self.tmp_dfs.clear();
            self.tmp_dfs.push(extra);
            'dfs: while let Some(x) = self.tmp_dfs.pop() {
                let mut found_in_iter: i64 = -1;
                let mut stop = false;
                exp_out_arcs(exp.g, a_orig, x, |xy| {
                    if stop {
                        return;
                    }
                    let y = exp_head(exp.g, a_orig, xy);
                    if self.reach[my_src].inside[y as usize] {
                        return;
                    }
                    if self.is_saturated(exp, a_orig, my_src, xy) {
                        return;
                    }
                    self.predecessor[my_src][y as usize] = xy;
                    self.reach[my_src].inside[y as usize] = true;
                    self.reach[my_src].count += 1;
                    if self.assim[my_tgt].inside[y as usize] {
                        found_in_iter = y as i64;
                        stop = true;
                        return;
                    }
                    self.tmp_dfs.push(y);
                });
                if found_in_iter >= 0 {
                    target_hit = found_in_iter;
                    break 'dfs;
                }
            }

            if target_hit >= 0 {
                let target = target_hit as u32;
                self.augment_along_path(exp, a_orig, my_src, target, pierced_side == SOURCE_SIDE);
                self.reset_reachable(my_src);
                was_flow_augmented = true;
            } else {
                break;
            }
        }

        if was_flow_augmented {
            self.reset_reachable(my_tgt);
            // No early exit here, unlike the my_src grow above: this needs the
            // full reachable set, not just the first augmenting path.
            let extra = match self.reach[my_tgt].extra.take() {
                Some(x) => x,
                None => return,
            };
            self.tmp_dfs.clear();
            self.tmp_dfs.push(extra);
            while let Some(x) = self.tmp_dfs.pop() {
                exp_out_arcs(exp.g, a_orig, x, |xy| {
                    let y = exp_head(exp.g, a_orig, xy);
                    if self.reach[my_tgt].inside[y as usize] {
                        return;
                    }
                    if self.is_saturated(exp, a_orig, my_tgt, xy) {
                        return;
                    }
                    self.predecessor[my_tgt][y as usize] = xy;
                    self.reach[my_tgt].inside[y as usize] = true;
                    self.reach[my_tgt].count += 1;
                    self.tmp_dfs.push(y);
                });
            }
        }
    }

    fn augment_along_path(
        &mut self,
        exp: &Exp,
        a_orig: u32,
        my_src: usize,
        target: u32,
        pierced_from_source: bool,
    ) {
        let mut x = target;
        while !self.assim[my_src].inside[x as usize] {
            let xy = self.predecessor[my_src][x as usize];
            debug_assert!(xy != NO_PRED, "predecessor chain broken");
            let back = exp_back(exp.g, a_orig, xy);
            if pierced_from_source {
                self.flow.increase(xy, back);
            } else {
                self.flow.decrease(xy, back);
            }
            x = exp_tail(exp.g, a_orig, xy);
        }
    }

    fn reset_reachable(&mut self, side: usize) {
        for (r, a) in self.reach[side]
            .inside
            .iter_mut()
            .zip(self.assim[side].inside.iter())
        {
            *r = *a;
        }
        self.reach[side].count = self.assim[side].count;
        self.reach[side].extra = self.assim[side].extra;
    }

    fn grow_assimilated_sets(&mut self, exp: &Exp, a_orig: u32) {
        let smaller = if self.reach[SOURCE_SIDE].count <= self.reach[TARGET_SIDE].count {
            SOURCE_SIDE
        } else {
            TARGET_SIDE
        };

        let extra = match self.assim[smaller].extra.take() {
            Some(x) => x,
            None => return,
        };

        self.tmp_dfs.clear();
        self.tmp_dfs.push(extra);
        while let Some(x) = self.tmp_dfs.pop() {
            exp_out_arcs(exp.g, a_orig, x, |xy| {
                let y = exp_head(exp.g, a_orig, xy);
                let f = self.flow.get(xy);
                if f != 0 {
                    self.front[smaller].push(xy);
                }
                if self.assim[smaller].inside[y as usize] {
                    return;
                }
                if self.is_saturated(exp, a_orig, smaller, xy) {
                    return;
                }
                self.assim[smaller].inside[y as usize] = true;
                self.assim[smaller].count += 1;
                self.tmp_dfs.push(y);
            });
        }

        let inside_ref = &self.assim[smaller].inside;
        self.front[smaller].retain(|&xy| !inside_ref[exp_head(exp.g, a_orig, xy) as usize]);
    }

    fn current_cut_side(&self) -> usize {
        // Arbitrary tie-break, chosen to match the ported reference implementation.
        let src_sat = self.reach[SOURCE_SIDE].count == self.assim[SOURCE_SIDE].count;
        let tgt_sat = self.reach[TARGET_SIDE].count == self.assim[TARGET_SIDE].count;
        if src_sat && (!tgt_sat || self.assim[SOURCE_SIDE].count <= self.assim[TARGET_SIDE].count) {
            SOURCE_SIDE
        } else {
            TARGET_SIDE
        }
    }

    fn current_cut(&self) -> &[u32] {
        &self.front[self.current_cut_side()]
    }
    fn current_smaller_size(&self) -> u32 {
        self.assim[self.current_cut_side()].count
    }

    fn is_on_smaller_side(&self, x: u32) -> bool {
        self.assim[self.current_cut_side()].inside[x as usize]
    }

    #[inline]
    fn score_pierce(&self, y: u32, side: usize, causes_aug: bool) -> i64 {
        let src_dist = self.node_dist[side][y as usize];
        let tgt_dist = self.node_dist[1 - side][y as usize];
        let mut score = (tgt_dist as i64).saturating_sub(src_dist as i64);
        if causes_aug {
            score = score.saturating_sub(1_000_000_000);
        }
        score
    }

    fn select_pierce_node(&self, exp: &Exp, a_orig: u32, side: usize) -> Option<u32> {
        let mut best = i64::MIN;
        let mut chosen: Option<u32> = None;
        for &xy in &self.front[side] {
            let y = exp_head(exp.g, a_orig, xy);
            if self.assim[1 - side].inside[y as usize] {
                continue;
            }
            let causes_aug = self.reach[1 - side].inside[y as usize];
            let s = self.score_pierce(y, side, causes_aug);
            if s > best {
                best = s;
                chosen = Some(y);
            }
        }
        chosen
    }

    fn does_next_advance_increase_cut(&self, exp: &Exp, a_orig: u32) -> bool {
        let side = self.current_cut_side();
        if self.assim[side].count >= n_exp(exp.g.n) / 2 {
            return true;
        }
        let py = self.select_pierce_node(exp, a_orig, side);
        match py {
            None => true,
            Some(y) => self.reach[1 - side].inside[y as usize],
        }
    }

    /// Returns false once no further cut is reachable.
    fn advance(&mut self, exp: &Exp, a_orig: u32) -> bool {
        debug_assert!(self.cut_available);
        let side = self.current_cut_side();
        if self.assim[side].count >= n_exp(exp.g.n) / 2 {
            self.cut_available = false;
            return false;
        }
        let py = self.select_pierce_node(exp, a_orig, side);
        let pierce = match py {
            Some(y) => y,
            None => {
                self.cut_available = false;
                return false;
            }
        };
        self.assim[side].set_extra(pierce);
        self.reach[side].set_extra(pierce);
        self.grow_reachable_sets(exp, a_orig, side);
        self.grow_assimilated_sets(exp, a_orig);
        self.cut_available = true;
        true
    }
}

pub(super) struct MultiCutter {
    cutters: Vec<BasicCutter>,
    current_id: usize,
    /// Snapshot from the last commit, not a live read of `cutters[current_id]`:
    /// intervening `BasicCutter::advance` calls move that cutter's
    /// smaller-side count before the next Pareto comparison runs.
    current_smaller: u32,
}

impl MultiCutter {
    pub(super) fn new(n_exp: u32, a_exp: u32, count: u32) -> Self {
        let mut cutters = Vec::with_capacity(count as usize);
        for _ in 0..count {
            cutters.push(BasicCutter::new(n_exp, a_exp));
        }
        MultiCutter {
            cutters,
            current_id: 0,
            current_smaller: 0,
        }
    }

    pub(super) fn init(&mut self, exp: &Exp, a_orig: u32, pairs: &[(u32, u32)]) {
        let n_exp_v = n_exp(exp.g.n);
        let a_exp_v = a_exp(exp.g.n, a_orig);
        while self.cutters.len() > pairs.len() {
            self.cutters.pop();
        }
        while self.cutters.len() < pairs.len() {
            self.cutters.push(BasicCutter::new(n_exp_v, a_exp_v));
        }

        for (i, &p) in pairs.iter().enumerate() {
            self.cutters[i].init(exp, a_orig, p);
            // Arbitrary: matches the reference implementation's default of
            // skipping non-maximum sides.
            let mut iter_guard = 0u32;
            loop {
                if self.cutters[i].does_next_advance_increase_cut(exp, a_orig) {
                    break;
                }
                if !self.cutters[i].advance(exp, a_orig) {
                    break;
                }
                iter_guard += 1;
                if iter_guard > 1_000_000 {
                    break;
                }
            }
        }

        let mut best_id = 0usize;
        let mut best_size = i64::MAX;
        let mut best_weight = 0u32;
        for (i, c) in self.cutters.iter().enumerate() {
            let s = c.current_cut().len() as i64;
            let w = c.current_smaller_size();
            if s < best_size || (s == best_size && w > best_weight) {
                best_id = i;
                best_size = s;
                best_weight = w;
            }
        }
        self.current_id = best_id;
        self.current_smaller = self.cutters[best_id].current_smaller_size();
    }

    pub(super) fn current_cut_size(&self) -> usize {
        self.cutters[self.current_id].current_cut().len()
    }
    pub(super) fn current_smaller_size(&self) -> u32 {
        self.current_smaller
    }
    pub(super) fn current_cut(&self) -> &[u32] {
        self.cutters[self.current_id].current_cut()
    }
    pub(super) fn is_on_smaller_side(&self, x: u32) -> bool {
        self.cutters[self.current_id].is_on_smaller_side(x)
    }

    pub(super) fn advance(&mut self, exp: &Exp, a_orig: u32) -> bool {
        if n_exp(exp.g.n) / 2 == self.current_smaller {
            return false;
        }

        let mut cur_size = self.current_cut_size();

        loop {
            for i in 0..self.cutters.len() {
                if !self.cutters[i].cut_available {
                    continue;
                }
                if self.cutters[i].current_cut().len() != cur_size {
                    continue;
                }
                let advanced = self.cutters[i].advance(exp, a_orig);
                if !advanced {
                    continue;
                }
                let mut iter_guard = 0u32;
                loop {
                    if self.cutters[i].does_next_advance_increase_cut(exp, a_orig) {
                        break;
                    }
                    if !self.cutters[i].advance(exp, a_orig) {
                        break;
                    }
                    iter_guard += 1;
                    if iter_guard > 1_000_000 {
                        break;
                    }
                }
            }

            let mut next_size = i64::MAX;
            for c in &self.cutters {
                if c.cut_available {
                    next_size = next_size.min(c.current_cut().len() as i64);
                }
            }
            if next_size == i64::MAX {
                return false;
            }

            let mut best_id = usize::MAX;
            let mut best_weight = 0u32;
            for (i, c) in self.cutters.iter().enumerate() {
                if !c.cut_available {
                    continue;
                }
                if c.current_cut().len() as i64 == next_size {
                    let w = c.current_smaller_size();
                    if w > best_weight {
                        best_id = i;
                        best_weight = w;
                    }
                }
            }
            debug_assert!(best_id != usize::MAX);

            cur_size = next_size as usize;
            // FlowCutter's Pareto rule: only commit once the smaller side is
            // strictly larger too, not merely once the cut size has grown.
            if best_weight <= self.current_smaller {
                continue;
            }

            self.current_id = best_id;
            self.current_smaller = best_weight;
            return true;
        }
    }
}
