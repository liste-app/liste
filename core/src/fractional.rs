//! Fractional indexing for manual order (Section 6).
//!
//! Positions are strings over a base-62 alphabet compared bytewise. A key
//! strictly between two neighbours always exists, so a move never rewrites
//! other rows; keys grow by one character per repeated insertion at the
//! same spot, and [`needs_rebalance`] says when a list should be rewritten
//! with evenly spaced short keys. No key ever ends in the lowest digit, so
//! a gap below any key always exists.

const DIGITS: &[u8; 62] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";

/// The key a fresh row gets when nothing else orders it.
pub const FIRST: &str = "V";

/// Keys longer than this suggest a rebalance of their list.
pub const REBALANCE_LENGTH: usize = 24;

fn digit(c: u8) -> usize {
    DIGITS.iter().position(|d| *d == c).unwrap_or(0)
}

fn at(key: &[u8], i: usize) -> usize {
    key.get(i).map(|c| digit(*c)).unwrap_or(0)
}

/// A key strictly between `before` and `after`. `None` means the open end.
/// Returns `None` if `before >= after`.
pub fn between(before: Option<&str>, after: Option<&str>) -> Option<String> {
    let lo = before.map(str::as_bytes).unwrap_or(b"");
    let hi = after.map(str::as_bytes);
    if let Some(hi) = hi
        && (lo >= hi || hi.is_empty())
    {
        return None;
    }
    let mut out: Vec<u8> = Vec::new();
    let mut i = 0;
    loop {
        let l = at(lo, i);
        let h = match hi {
            Some(hi) => {
                if i < hi.len() {
                    digit(hi[i])
                } else {
                    // `hi` is exhausted: it ends in the lowest digit, which
                    // valid keys never do, so there is no gap.
                    return None;
                }
            }
            None => 62,
        };
        if h - l > 1 {
            out.push(DIGITS[(l + h) / 2]);
            return Some(String::from_utf8(out).expect("ascii"));
        }
        // Digits adjacent or equal: copy the low digit and descend.
        out.push(DIGITS[l]);
        if h == l {
            i += 1;
            continue;
        }
        // h == l + 1: everything after this position on the low side is the
        // lower bound; the upper side is now open (all 62s).
        i += 1;
        loop {
            let l = at(lo, i);
            if l < 61 {
                out.push(DIGITS[(l + 62) / 2]);
                return Some(String::from_utf8(out).expect("ascii"));
            }
            out.push(DIGITS[l]);
            i += 1;
        }
    }
}

/// Evenly spaced keys for `n` rows, shortest length that fits.
pub fn rebalanced(n: usize) -> Vec<String> {
    if n == 0 {
        return Vec::new();
    }
    let mut len = 1usize;
    let mut capacity = 62usize;
    while capacity < n + 2 {
        len += 1;
        capacity *= 62;
    }
    let step = capacity / (n + 1);
    (1..=n)
        .map(|k| {
            let mut v = step * k;
            let mut s = vec![b'0'; len];
            for slot in s.iter_mut().rev() {
                *slot = DIGITS[v % 62];
                v /= 62;
            }
            // Trailing lowest digits carry no order and would close the gap
            // below the key.
            while s.len() > 1 && s.last() == Some(&b'0') {
                s.pop();
            }
            String::from_utf8(s).expect("ascii")
        })
        .collect()
}

/// Whether any of these keys is long enough that the list should be rewritten.
pub fn needs_rebalance<'a>(keys: impl IntoIterator<Item = &'a str>) -> bool {
    keys.into_iter().any(|k| k.len() > REBALANCE_LENGTH)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn between_open_ends() {
        let mid = between(None, None).unwrap();
        assert!(!mid.is_empty());
        let after = between(Some(&mid), None).unwrap();
        assert!(after.as_str() > mid.as_str());
        let before = between(None, Some(&mid)).unwrap();
        assert!(before.as_str() < mid.as_str());
    }

    #[test]
    fn repeated_insertion_at_the_same_gap_always_fits() {
        let lo = "A".to_string();
        let hi = "B".to_string();
        let mut prev = lo.clone();
        for _ in 0..200 {
            let k = between(Some(&prev), Some(&hi)).unwrap();
            assert!(k.as_str() > prev.as_str() && k.as_str() < hi.as_str());
            prev = k;
        }
        assert!(needs_rebalance([prev.as_str()]));
    }

    #[test]
    fn repeated_prepend_and_append() {
        let mut first = FIRST.to_string();
        let mut last = FIRST.to_string();
        for _ in 0..200 {
            let f = between(None, Some(&first)).unwrap();
            assert!(f.as_str() < first.as_str());
            first = f;
            let l = between(Some(&last), None).unwrap();
            assert!(l.as_str() > last.as_str());
            last = l;
        }
    }

    #[test]
    fn invalid_ranges_yield_none() {
        assert_eq!(between(Some("B"), Some("A")), None);
        assert_eq!(between(Some("A"), Some("A")), None);
        assert_eq!(
            between(Some("A"), Some("A0")),
            None,
            "no valid key ends in 0"
        );
    }

    #[test]
    fn rebalanced_keys_are_short_sorted_and_distinct() {
        let keys = rebalanced(1000);
        assert_eq!(keys.len(), 1000);
        assert!(keys.windows(2).all(|w| w[0] < w[1]));
        assert!(keys.iter().all(|k| k.len() <= 2 && !k.ends_with('0')));
        for pair in keys.windows(2) {
            assert!(between(Some(&pair[0]), Some(&pair[1])).is_some());
        }
        assert!(between(None, Some(&keys[0])).is_some());
        assert!(!needs_rebalance(keys.iter().map(String::as_str)));
        assert_eq!(rebalanced(0).len(), 0);
        assert_eq!(rebalanced(1)[0].len(), 1);
    }
}
