//! Keeping the best of several candidates.
//!
//! Every construction that builds more than one vtree and returns one picks the
//! same way: lex-min over a sort key, with a tie going to whichever candidate
//! was offered first. The rule is written once here because two of those
//! constructions build the SAME candidates in the same order — a tie broken the
//! other way in one of them would silently return a different tree than the
//! other for the same formula.
//!
//! Candidates are offered one at a time rather than collected: the expensive
//! ones are whole vtrees, and a construction that keeps one keeps exactly one.

/// A running minimum over candidates offered one at a time.
pub(crate) struct BestBy<T, K> {
    best: Option<(T, K)>,
}

impl<T, K: PartialOrd> BestBy<T, K> {
    /// An accumulator holding no candidate yet.
    pub(crate) fn new() -> Self {
        BestBy { best: None }
    }

    /// Offer a candidate under `key`. It is kept only if `key` is strictly below
    /// the incumbent's, so the earliest of equally-keyed candidates wins.
    pub(crate) fn offer(&mut self, item: T, key: K) {
        if self.best.as_ref().is_none_or(|(_, bk)| key < *bk) {
            self.best = Some((item, key));
        }
    }

    /// Whether anything has been offered yet. A sweep that may abandon itself
    /// asks this so it never stops before it holds a candidate.
    pub(crate) fn has_candidate(&self) -> bool {
        self.best.is_some()
    }

    /// The winning candidate and its key, or `None` if nothing was offered.
    pub(crate) fn into_best(self) -> Option<(T, K)> {
        self.best
    }
}

/// [`BestBy`] over an iterator, for a caller that already has the candidates in
/// hand: lex-min by `key`, first occurrence winning ties, which is what
/// `Iterator::min_by_key` does.
pub(crate) fn select_first_min<T, K: PartialOrd>(
    items: impl IntoIterator<Item = T>,
    key: impl Fn(&T) -> K,
) -> Option<T> {
    let mut best = BestBy::new();
    for item in items {
        let k = key(&item);
        best.offer(item, k);
    }
    best.into_best().map(|(item, _)| item)
}
