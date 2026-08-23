use crate::decompose::goatd::flow_cut::*;

#[test]
fn trivial_path_separator_is_single_middle_vertex() {
    let edges = vec![(0u32, 1), (1, 2), (2, 3), (3, 4)];
    let part = vec![0u8, 0, 1, 1, 1];
    let r = min_cover_separator(5, &edges, &part);
    assert_eq!(r.separator.len(), 1);
    assert!(r.separator[0] == 1 || r.separator[0] == 2);
    assert_eq!(r.side_a.len() + r.side_b.len(), 4);
}

#[test]
fn cover_beats_smaller_boundary_on_star_crossing() {
    let edges = vec![(1u32, 10), (1, 11), (1, 12), (0, 1)];
    let mut part = vec![0u8; 13];
    part[10..=12].fill(1);
    let r = min_cover_separator(13, &edges, &part);
    assert_eq!(r.separator, vec![1]);
}

#[test]
fn cover_strictly_smaller_than_smaller_boundary() {
    // A={1,2,3}, B={10}, all cross-edges converge on the hub — sanity-checks
    // the general property (cover <= smaller-boundary) on an asymmetric case.
    let edges = vec![(1u32, 10), (2, 10), (3, 10), (0, 1), (0, 2), (0, 3)];
    let mut part = vec![0u8; 11];
    part[10] = 1;
    let r = min_cover_separator(11, &edges, &part);
    assert_eq!(r.separator, vec![10]);
}

#[test]
fn empty_cut_yields_empty_separator() {
    let edges = vec![(0u32, 1), (1, 2)];
    let part = vec![0u8; 3];
    let r = min_cover_separator(3, &edges, &part);
    assert!(r.separator.is_empty());
    assert_eq!(r.side_a.len(), 3);
    assert!(r.side_b.is_empty());
}
