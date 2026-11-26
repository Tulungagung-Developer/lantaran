/// A host name pattern where `*` stands for any run of characters.
///
/// Matching ignores ASCII case, since host names are case-insensitive.
/// `*.example.com` matches `api.example.com` but not `example.com` itself;
/// list both when both are wanted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostPattern {
    pattern: String,
}

impl HostPattern {
    pub fn new(pattern: impl Into<String>) -> Self {
        Self {
            pattern: pattern.into().trim().to_ascii_lowercase(),
        }
    }

    pub fn as_str(&self) -> &str {
        &self.pattern
    }

    pub fn matches(&self, host: &str) -> bool {
        let pattern = self.pattern.as_bytes();
        let host = host.as_bytes();

        let (mut p, mut h) = (0, 0);
        // Position of the last `*` seen, and the host position it was tried at.
        let mut backtrack: Option<(usize, usize)> = None;

        while h < host.len() {
            if p < pattern.len() && pattern[p] == b'*' {
                backtrack = Some((p, h));
                p += 1;
            } else if p < pattern.len() && pattern[p] == host[h].to_ascii_lowercase() {
                p += 1;
                h += 1;
            } else if let Some((star, tried)) = backtrack {
                // Let the last `*` swallow one more character and retry.
                backtrack = Some((star, tried + 1));
                p = star + 1;
                h = tried + 1;
            } else {
                return false;
            }
        }

        pattern[p..].iter().all(|&b| b == b'*')
    }
}

impl From<&str> for HostPattern {
    fn from(pattern: &str) -> Self {
        Self::new(pattern)
    }
}

impl From<String> for HostPattern {
    fn from(pattern: String) -> Self {
        Self::new(pattern)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_host() {
        let pattern = HostPattern::new("api.example.com");
        assert!(pattern.matches("api.example.com"));
        assert!(!pattern.matches("example.com"));
        assert!(!pattern.matches("api.example.com.evil.test"));
    }

    #[test]
    fn ignores_case_and_surrounding_space() {
        let pattern = HostPattern::new("  API.Example.com ");
        assert!(pattern.matches("api.EXAMPLE.com"));
    }

    #[test]
    fn leading_wildcard_needs_a_subdomain() {
        let pattern = HostPattern::new("*.example.com");
        assert!(pattern.matches("api.example.com"));
        assert!(pattern.matches("a.b.example.com"));
        assert!(!pattern.matches("example.com"));
        assert!(!pattern.matches("notexample.com"));
    }

    #[test]
    fn wildcard_in_the_middle_and_at_the_end() {
        assert!(HostPattern::new("api-*.example.com").matches("api-staging.example.com"));
        assert!(!HostPattern::new("api-*.example.com").matches("api.example.com"));
        assert!(HostPattern::new("10.0.*").matches("10.0.2.2"));
    }

    #[test]
    fn lone_wildcard_matches_everything() {
        let pattern = HostPattern::new("*");
        assert!(pattern.matches("anything.test"));
        assert!(pattern.matches(""));
    }

    #[test]
    fn wildcard_backtracks_over_repeated_text() {
        assert!(HostPattern::new("*.com.example.com").matches("a.com.com.example.com"));
        assert!(HostPattern::new("*a*b").matches("xxaxxaxxb"));
        assert!(!HostPattern::new("*a*b").matches("xxaxxaxx"));
    }

    #[test]
    fn empty_pattern_matches_only_an_empty_host() {
        let pattern = HostPattern::new("");
        assert!(pattern.matches(""));
        assert!(!pattern.matches("example.com"));
    }
}
