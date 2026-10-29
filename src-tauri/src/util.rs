//! Small helpers shared across modules.

/// Current Unix time in seconds (0 if the clock is before the epoch).
pub fn unix_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Classic edit distance over chars (insert / delete / substitute = 1).
pub fn levenshtein(a: &[char], b: &[char]) -> usize {
    if a == b {
        return 0;
    }
    if a.is_empty() {
        return b.len();
    }
    if b.is_empty() {
        return a.len();
    }
    let n = b.len();
    let mut prev: Vec<usize> = (0..=n).collect();
    let mut curr: Vec<usize> = vec![0; n + 1];
    for i in 1..=a.len() {
        curr[0] = i;
        let ca = a[i - 1];
        for j in 1..=n {
            let cost = if ca == b[j - 1] { 0 } else { 1 };
            curr[j] = (curr[j - 1] + 1).min(prev[j] + 1).min(prev[j - 1] + cost);
        }
        std::mem::swap(&mut prev, &mut curr);
    }
    prev[n]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn levenshtein_matches_known_distances() {
        let chars = |s: &str| s.chars().collect::<Vec<_>>();
        assert_eq!(levenshtein(&[], &[]), 0);
        assert_eq!(levenshtein(&[], &chars("abc")), 3);
        assert_eq!(levenshtein(&chars("kitten"), &chars("sitting")), 3);
    }

    #[test]
    fn unix_secs_returns_a_plausible_current_timestamp() {
        let secs = unix_secs();
        assert!(secs > 1_700_000_000, "secs={secs}");
    }
}
