//! "Did you mean" suggestions by edit distance.

/// Optimal string alignment distance (Levenshtein plus adjacent
/// transpositions, so `mosue` is one edit from `mouse`).
pub(crate) fn distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let w = b.len() + 1;
    let mut d = vec![0usize; (a.len() + 1) * w];
    for i in 0..=a.len() {
        d[i * w] = i;
    }
    for (j, cell) in d.iter_mut().enumerate().take(w) {
        *cell = j;
    }
    for i in 1..=a.len() {
        for j in 1..=b.len() {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            let mut v = (d[(i - 1) * w + j] + 1).min(d[i * w + j - 1] + 1).min(d[(i - 1) * w + j - 1] + cost);
            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                v = v.min(d[(i - 2) * w + j - 2] + 1);
            }
            d[i * w + j] = v;
        }
    }
    d[a.len() * w + b.len()]
}

/// The closest candidate, if it is close enough to be a plausible typo.
pub(crate) fn suggest<'a>(word: &str, candidates: impl IntoIterator<Item = &'a str>) -> Option<&'a str> {
    let len = word.chars().count();
    let limit = (len / 3).clamp(1, 3);
    let mut best: Option<(usize, &str)> = None;
    for c in candidates {
        let d = distance(&word.to_ascii_lowercase(), &c.to_ascii_lowercase());
        if d <= limit && best.is_none_or(|(bd, _)| d < bd) {
            best = Some((d, c));
        }
    }
    best.map(|(_, c)| c)
}

/// `", did you mean `x`?"` or nothing.
pub(crate) fn did_you_mean<'a>(word: &str, candidates: impl IntoIterator<Item = &'a str>, quote: (&str, &str)) -> String {
    match suggest(word, candidates) {
        Some(s) => format!(", did you mean {}{s}{}?", quote.0, quote.1),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distances() {
        assert_eq!(distance("mouse", "mouse"), 0);
        assert_eq!(distance("mouze", "mouse"), 1);
        assert_eq!(distance("mosue", "mouse"), 1);
        assert_eq!(distance("", "abc"), 3);
        assert_eq!(distance("kitten", "sitting"), 3);
    }

    #[test]
    fn suggestions() {
        let c = ["mouse", "mode-keys", "base-index", "pane-base-index"];
        assert_eq!(suggest("mouze", c), Some("mouse"));
        assert_eq!(suggest("base-idx", c), Some("base-index"));
        assert_eq!(suggest("zzzzzz", c), None);
        assert_eq!(suggest("x", c), None);
    }
}
