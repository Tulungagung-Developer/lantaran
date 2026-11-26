use crate::UrlPattern;

/// Answers matching requests from a local file instead of the server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MapLocal {
    pub pattern: UrlPattern,
    /// `None` matches every method.
    pub method: Option<String>,
    /// Read at request time, so edits apply to the next request.
    pub file: String,
    pub status: u16,
    /// `None` means guess from the file's extension.
    pub content_type: Option<String>,
}

/// Sends matching requests to a different server or path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MapRemote {
    pub pattern: UrlPattern,
    /// `None` matches every method.
    pub method: Option<String>,
    /// Each part that is `Some` replaces that part of the URL; the rest is
    /// kept, including the query.
    pub scheme: Option<String>,
    pub host: Option<String>,
    pub port: Option<u16>,
    pub path: Option<String>,
}

/// Every mapping rule, checked in order. The first match wins, and Map
/// Local is checked before Map Remote.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MapRules {
    pub local: Vec<MapLocal>,
    pub remote: Vec<MapRemote>,
}

impl MapRules {
    pub fn local_for(&self, method: &str, url: &str) -> Option<&MapLocal> {
        self.local
            .iter()
            .find(|rule| method_matches(&rule.method, method) && rule.pattern.matches(url))
    }

    pub fn remote_for(&self, method: &str, url: &str) -> Option<&MapRemote> {
        self.remote
            .iter()
            .find(|rule| method_matches(&rule.method, method) && rule.pattern.matches(url))
    }
}

fn method_matches(rule: &Option<String>, method: &str) -> bool {
    rule.as_deref()
        .is_none_or(|wanted| wanted.eq_ignore_ascii_case(method))
}

/// Guesses a content type from a file name, for Map Local files.
pub fn content_type_for(file: &str) -> &'static str {
    let extension = file
        .rsplit_once('.')
        .map(|(_, ext)| ext.to_ascii_lowercase())
        .unwrap_or_default();
    match extension.as_str() {
        "json" => "application/json",
        "html" | "htm" => "text/html; charset=utf-8",
        "txt" | "log" => "text/plain; charset=utf-8",
        "xml" => "application/xml",
        "js" => "text/javascript",
        "css" => "text/css",
        "csv" => "text/csv",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "pdf" => "application/pdf",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn local(pattern: &str, method: Option<&str>, file: &str) -> MapLocal {
        MapLocal {
            pattern: pattern.into(),
            method: method.map(str::to_owned),
            file: file.to_owned(),
            status: 200,
            content_type: None,
        }
    }

    #[test]
    fn first_matching_rule_wins() {
        let rules = MapRules {
            local: vec![
                local("https://api.test/users/*", None, "first.json"),
                local("https://api.test/*", None, "second.json"),
            ],
            remote: vec![],
        };

        assert_eq!(
            rules
                .local_for("GET", "https://api.test/users/1")
                .unwrap()
                .file,
            "first.json"
        );
        assert_eq!(
            rules
                .local_for("GET", "https://api.test/other")
                .unwrap()
                .file,
            "second.json"
        );
        assert!(rules.local_for("GET", "https://other.test/").is_none());
    }

    #[test]
    fn method_is_matched_when_set() {
        let rules = MapRules {
            local: vec![local("https://api.test/*", Some("post"), "post.json")],
            remote: vec![],
        };

        assert!(rules.local_for("POST", "https://api.test/x").is_some());
        assert!(rules.local_for("GET", "https://api.test/x").is_none());
    }

    #[test]
    fn content_type_is_guessed_from_the_extension() {
        assert_eq!(content_type_for("users.JSON"), "application/json");
        assert_eq!(content_type_for("/tmp/logo.png"), "image/png");
        assert_eq!(content_type_for("no-extension"), "application/octet-stream");
    }
}
