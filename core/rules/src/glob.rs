/// Whether `text` matches `pattern`, where `*` stands for any run of
/// characters, including none. Comparison is byte for byte; callers
/// normalise case first where it does not matter.
pub(crate) fn glob_match(pattern: &[u8], text: &[u8]) -> bool {
    let (mut p, mut t) = (0, 0);
    // Position of the last `*` seen, and the text position it was tried at.
    let mut backtrack: Option<(usize, usize)> = None;

    while t < text.len() {
        if p < pattern.len() && pattern[p] == b'*' {
            backtrack = Some((p, t));
            p += 1;
        } else if p < pattern.len() && pattern[p] == text[t] {
            p += 1;
            t += 1;
        } else if let Some((star, tried)) = backtrack {
            // Let the last `*` swallow one more character and retry.
            backtrack = Some((star, tried + 1));
            p = star + 1;
            t = tried + 1;
        } else {
            return false;
        }
    }

    pattern[p..].iter().all(|&b| b == b'*')
}
