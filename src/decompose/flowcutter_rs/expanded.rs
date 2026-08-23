//! The vertex-split graph, addressed arithmetically: never materialized, every
//! node/arc/capacity/reverse-arc is a closed-form function of the original
//! graph's indices, computed here.
//!
//! Node IDs:
//!   v_in  = 2*v
//!   v_out = 2*v + 1
//! Arc IDs:
//!   inter [0, 2*a_orig):     2*orig_a + tail_out_flag
//!   intra [2*a_orig, ...):   2*(a_orig + v) + tail_out_flag
//! Capacities:
//!   intra cap = 1 - tail_out_flag    (v_in→v_out has cap 1, v_out→v_in has cap 0)
//!   inter cap = tail_out_flag        (u_out→v_in has cap 1, u_in→v_out has cap 0)

use super::*;

#[inline]
pub(super) fn n_exp(n_orig: u32) -> u32 {
    2 * n_orig
}

#[inline]
pub(super) fn a_exp(n_orig: u32, a_orig: u32) -> u32 {
    2 * (a_orig + n_orig)
}

#[inline]
pub(super) fn is_intra(arc: u32, a_orig: u32) -> bool {
    arc >= 2 * a_orig
}

#[inline]
pub(super) fn exp_node_to_orig(x: u32) -> u32 {
    x >> 1
}

#[inline]
pub(super) fn exp_node_out_flag(x: u32) -> u32 {
    x & 1
}

#[inline]
pub(super) fn orig_node_to_exp(v: u32, out_flag: bool) -> u32 {
    2 * v + (out_flag as u32)
}

#[inline]
pub(super) fn intra_to_orig_node(arc: u32, a_orig: u32) -> u32 {
    (arc / 2) - a_orig
}

#[inline]
pub(super) fn arc_tail_out_flag(arc: u32) -> u32 {
    arc & 1
}

#[inline]
pub(super) fn inter_to_orig_arc(arc: u32) -> u32 {
    arc / 2
}

pub(super) fn exp_tail(g: &OrigGraph, a_orig: u32, arc: u32) -> u32 {
    let flag = arc_tail_out_flag(arc);
    if is_intra(arc, a_orig) {
        let v = intra_to_orig_node(arc, a_orig);
        2 * v + flag
    } else {
        let oa = inter_to_orig_arc(arc) as usize;
        2 * g.tail[oa] + flag
    }
}

pub(super) fn exp_head(g: &OrigGraph, a_orig: u32, arc: u32) -> u32 {
    let flag = arc_tail_out_flag(arc);
    let head_flag = 1 - flag;
    if is_intra(arc, a_orig) {
        let v = intra_to_orig_node(arc, a_orig);
        2 * v + head_flag
    } else {
        let oa = inter_to_orig_arc(arc) as usize;
        2 * g.head[oa] + head_flag
    }
}

pub(super) fn exp_back(g: &OrigGraph, a_orig: u32, arc: u32) -> u32 {
    if is_intra(arc, a_orig) {
        arc ^ 1
    } else {
        let oa = inter_to_orig_arc(arc) as usize;
        let flag = arc_tail_out_flag(arc);
        2 * g.back_arc[oa] + (1 - flag)
    }
}

pub(super) fn exp_capacity(a_orig: u32, arc: u32) -> i8 {
    if is_intra(arc, a_orig) {
        1 - (arc & 1) as i8
    } else {
        (arc & 1) as i8
    }
}

/// Iterate the expanded out-arcs of node `x` (intra arc first, then inter
/// arcs in the order of `out_arcs(orig)`).
pub(super) fn exp_out_arcs<F: FnMut(u32)>(g: &OrigGraph, a_orig: u32, x: u32, mut f: F) {
    let v = exp_node_to_orig(x);
    let flag = exp_node_out_flag(x);
    let intra = 2 * (a_orig + v) + flag;
    f(intra);
    for &oa in g.out_arcs(v) {
        f(2 * oa + flag);
    }
}
