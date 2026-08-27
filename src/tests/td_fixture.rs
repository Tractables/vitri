//! Tree-decomposition fixtures shared by vitri's construction tests.

use crate::decompose::TreeDecomposition;
use crate::tests::common::make_td;

/// A three-bag path decomposition of six variables, sharing two variables
/// across the first join and one across the second.
pub(crate) fn make_test_td() -> TreeDecomposition {
    make_td(
        vec![vec![0, 1, 2], vec![1, 2, 3], vec![3, 4, 5]],
        vec![(0, 1), (1, 2)],
        6,
    )
}
