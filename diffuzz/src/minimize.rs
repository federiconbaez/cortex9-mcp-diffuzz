//! Delta debugging (ddmin, Zeller & Hildebrandt) over step sequences.

/// Smallest subsequence of `items` (1-minimal) for which `test` holds.
/// `test(items)` must be true on entry.
pub fn ddmin<T: Clone>(items: &[T], mut test: impl FnMut(&[T]) -> bool) -> Vec<T> {
    let mut cur: Vec<T> = items.to_vec();
    let mut n = 2usize;
    while cur.len() >= 2 {
        let chunk = cur.len().div_ceil(n);
        let mut reduced = false;
        // Try each complement (remove one chunk).
        let mut start = 0;
        while start < cur.len() {
            let end = (start + chunk).min(cur.len());
            let candidate: Vec<T> = cur[..start].iter().chain(&cur[end..]).cloned().collect();
            if !candidate.is_empty() && test(&candidate) {
                cur = candidate;
                n = (n - 1).max(2);
                reduced = true;
                break;
            }
            start = end;
        }
        if !reduced {
            if n >= cur.len() {
                break;
            }
            n = (n * 2).min(cur.len());
        }
    }
    cur
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_the_two_needed_elements() {
        let items: Vec<u32> = (0..40).collect();
        let mut calls = 0;
        let out = ddmin(&items, |c| {
            calls += 1;
            c.contains(&7) && c.contains(&31)
        });
        assert_eq!(out, vec![7, 31]);
        assert!(calls < 200, "{calls} calls");
    }

    #[test]
    fn keeps_order_dependent_pairs() {
        // Failure needs a 3 somewhere before a 9.
        let items = vec![9, 1, 3, 4, 9, 2];
        let out = ddmin(&items, |c| {
            let p3 = c.iter().position(|x| *x == 3);
            let p9 = c.iter().rposition(|x| *x == 9);
            matches!((p3, p9), (Some(a), Some(b)) if a < b)
        });
        assert_eq!(out, vec![3, 9]);
    }

    #[test]
    fn single_element_stays() {
        assert_eq!(ddmin(&[5], |_| true), vec![5]);
    }
}
