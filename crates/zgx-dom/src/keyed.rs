//! The longest increasing subsequence, for moving the fewest rows when
//! keyed children reorder: rows whose old places increase along the new
//! order stay where they are, and only the others move.

/// Marks in `keep` the entries of `sources` (old places, `u32::MAX` for new
/// rows) in one longest strictly increasing run. O(n log n).
pub(crate) fn longest_increasing(sources: &[u32], keep: &mut Vec<bool>) {
    keep.clear();
    keep.resize(sources.len(), false);
    // tails[k]: the index of the smallest last entry of an increasing run
    // of length k + 1; previous[i]: the entry before i in its run.
    let mut tails: Vec<u32> = Vec::new();
    let mut previous: Vec<u32> = vec![u32::MAX; sources.len()];
    for (i, &source) in sources.iter().enumerate() {
        if source == u32::MAX {
            continue;
        }
        let at = tails.partition_point(|&t| sources[t as usize] < source);
        if at > 0 {
            previous[i] = tails[at - 1];
        }
        if at == tails.len() {
            tails.push(i as u32);
        } else {
            tails[at] = i as u32;
        }
    }
    let mut at = tails.last().copied().unwrap_or(u32::MAX);
    while at != u32::MAX {
        keep[at as usize] = true;
        at = previous[at as usize];
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wasm_bindgen_test::wasm_bindgen_test as test;

    fn kept(sources: &[u32]) -> Vec<u32> {
        let mut keep = Vec::new();
        longest_increasing(sources, &mut keep);
        sources
            .iter()
            .zip(&keep)
            .filter(|(_, k)| **k)
            .map(|(s, _)| *s)
            .collect()
    }

    #[test]
    fn keeps_the_longest_run() {
        assert_eq!(kept(&[0, 1, 2, 3]), vec![0, 1, 2, 3]);
        assert_eq!(kept(&[3, 1, 2, 0]).len(), 2);
        // A swap of the second and second-to-last moves two.
        assert_eq!(kept(&[0, 4, 2, 3, 1, 5]).len(), 4);
        assert_eq!(kept(&[u32::MAX, 0, u32::MAX, 1]), vec![0, 1]);
        assert!(kept(&[]).is_empty());
    }
}
