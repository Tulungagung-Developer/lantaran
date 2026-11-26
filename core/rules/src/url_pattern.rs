/// A pattern for whole URLs, where `*` stands for any run of characters.
///
/// - The scheme and host ignore case; the path and query do not.
/// - Without a scheme (`api.example.com/v1/*`) it matches both `http` and
///   `https`.
/// - Without a path (`https://api.example.com`) it matches every path on
///   that host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UrlPattern {
    original: String,
    /// Normalised: scheme and host lowercased, and `/*` added when there was
    /// no path.
    pattern: String,
    has_scheme: bool,
}

impl UrlPattern {
    pub fn new(pattern: impl Into<String>) -> Self {
        let original: String = pattern.into().trim().to_owned();
        let has_scheme = original.contains("://");
        let (prefix, rest) = match original.split_once("://") {
            Some((scheme, rest)) => (format!("{}://", scheme.to_ascii_lowercase()), rest),
            None => (String::new(), original.as_str()),
        };
        let (host, path) = match rest.find('/') {
            Some(slash) => (&rest[..slash], &rest[slash..]),
            None => (rest, "/*"),
        };

        Self {
            pattern: format!("{prefix}{}{path}", host.to_ascii_lowercase()),
            original,
            has_scheme,
        }
    }

    pub fn as_str(&self) -> &str {
        &self.original
    }

    pub fn matches(&self, url: &str) -> bool {
        let normalised = normalise(url);
        let text = if self.has_scheme {
            normalised.as_str()
        } else {
            normalised
                .split_once("://")
                .map_or(normalised.as_str(), |(_, rest)| rest)
        };
        crate::glob::glob_match(self.pattern.as_bytes(), text.as_bytes())
    }
}

/// Lowercases the scheme and host of a URL and gives it a path, so
/// `HTTPS://Example.com` compares equal to `https://example.com/`.
fn normalise(url: &str) -> String {
    let Some((scheme, rest)) = url.split_once("://") else {
        return url.to_owned();
    };
    let (host, path) = match rest.find(['/', '?']) {
        Some(at) if rest.as_bytes()[at] == b'/' => (&rest[..at], rest[at..].to_owned()),
        Some(at) => (&rest[..at], format!("/{}", &rest[at..])),
        None => (rest, "/".to_owned()),
    };
    format!(
        "{}://{}{path}",
        scheme.to_ascii_lowercase(),
        host.to_ascii_lowercase()
    )
}

impl From<&str> for UrlPattern {
    fn from(pattern: &str) -> Self {
        Self::new(pattern)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_url() {
        let pattern = UrlPattern::new("https://api.example.com/v1/users");
        assert!(pattern.matches("https://api.example.com/v1/users"));
        assert!(!pattern.matches("https://api.example.com/v1/users/1"));
        assert!(!pattern.matches("http://api.example.com/v1/users"));
    }

    #[test]
    fn wildcard_in_the_path() {
        let pattern = UrlPattern::new("https://api.example.com/v1/users/*");
        assert!(pattern.matches("https://api.example.com/v1/users/42"));
        assert!(pattern.matches("https://api.example.com/v1/users/42?expand=true"));
        assert!(!pattern.matches("https://api.example.com/v2/users/42"));
    }

    #[test]
    fn scheme_and_host_ignore_case_but_the_path_does_not() {
        let pattern = UrlPattern::new("HTTPS://API.Example.com/Users");
        assert!(pattern.matches("https://api.example.com/Users"));
        assert!(!pattern.matches("https://api.example.com/users"));
    }

    #[test]
    fn no_scheme_matches_http_and_https() {
        let pattern = UrlPattern::new("api.example.com/v1/*");
        assert!(pattern.matches("http://api.example.com/v1/a"));
        assert!(pattern.matches("https://api.example.com/v1/a"));
    }

    #[test]
    fn no_path_matches_the_whole_host() {
        let pattern = UrlPattern::new("https://api.example.com");
        assert!(pattern.matches("https://api.example.com/"));
        assert!(pattern.matches("https://api.example.com/anything?q=1"));
        assert!(pattern.matches("https://api.example.com"));
        assert!(!pattern.matches("https://api.example.com.evil.test/"));
    }

    #[test]
    fn wildcard_host() {
        let pattern = UrlPattern::new("https://*.example.com/graphql");
        assert!(pattern.matches("https://api.example.com/graphql"));
        assert!(!pattern.matches("https://example.com/graphql"));
    }

    #[test]
    fn a_port_is_part_of_the_host() {
        let pattern = UrlPattern::new("http://localhost:3000/*");
        assert!(pattern.matches("http://localhost:3000/api"));
        assert!(!pattern.matches("http://localhost:4000/api"));
    }

    #[test]
    fn a_query_without_a_path_still_matches_by_path() {
        let pattern = UrlPattern::new("https://example.com/*");
        assert!(pattern.matches("https://example.com?q=1"));
    }
}
