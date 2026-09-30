/// Match quality of `key` (lower-cased) against `pat` (lower-cased); 0 = no match.
pub(crate) fn quality(key: &str, pat: &str) -> u8 {
    if pat.is_empty() || key.starts_with(pat) {
        4
    } else if segment_match(key.as_bytes(), pat.as_bytes()) {
        3
    } else if key.contains(pat) {
        2
    } else if subsequence(key.as_bytes(), pat.as_bytes()) {
        1
    } else {
        0
    }
}

/// Higher is better: match quality dominates, then context group (lower = more relevant), then length.
pub(crate) fn score(quality: u8, group: u8, len: usize) -> i64 {
    quality as i64 * 1_000_000 - group as i64 * 10_000 - len.min(9_999) as i64
}

fn is_sep(b: u8) -> bool {
    !(b.is_ascii_alphanumeric() || b >= 0x80)
}

/// `pat` is a concatenation of prefixes of successive name segments (`ui` → `user_id`).
fn segment_match(key: &[u8], pat: &[u8]) -> bool {
    let mut starts = [0usize; 16];
    let mut n = 0;
    for i in 0..key.len() {
        if n == starts.len() {
            break;
        }
        if !is_sep(key[i]) && (i == 0 || is_sep(key[i - 1])) {
            starts[n] = i;
            n += 1;
        }
    }
    segments_from(key, &starts[..n], pat)
}

fn segments_from(key: &[u8], starts: &[usize], pat: &[u8]) -> bool {
    if pat.is_empty() {
        return true;
    }
    for (si, &s) in starts.iter().enumerate() {
        let seg = &key[s..];
        let common = seg.iter().zip(pat).take_while(|(a, b)| a == b && !is_sep(**a)).count();
        for k in (1..=common).rev() {
            if segments_from(key, &starts[si + 1..], &pat[k..]) {
                return true;
            }
        }
    }
    false
}

fn subsequence(key: &[u8], pat: &[u8]) -> bool {
    let mut it = key.iter();
    pat.iter().all(|p| it.any(|k| k == p))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quality_levels_are_ordered_by_match_strength() {
        assert_eq!(quality("user_id", "user"), 4);
        assert_eq!(quality("user_id", "ui"), 3);
        assert_eq!(quality("user_id", "id"), 3);
        assert_eq!(quality("orders", "der"), 2);
        assert_eq!(quality("user_id", "urd"), 1);
        assert_eq!(quality("user_id", "xyz"), 0);
        assert_eq!(quality("anything", ""), 4);
    }

    #[test]
    fn segment_prefixes_must_follow_segment_order() {
        assert_eq!(quality("order_items", "oi"), 3);
        assert_ne!(quality("order_items", "io"), 3);
    }

    #[test]
    fn score_prefers_quality_then_group_then_length() {
        assert!(score(4, 5, 30) > score(3, 0, 1));
        assert!(score(4, 0, 30) > score(4, 1, 1));
        assert!(score(4, 0, 3) > score(4, 0, 4));
    }
}
