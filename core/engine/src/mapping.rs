//! Map Local and Map Remote: answering a request from a file, or sending it
//! to a different server.

use http_body_util::Full;
use hudsucker::{
    Body,
    hyper::{
        Response, StatusCode, Uri,
        header::{CACHE_CONTROL, CONTENT_TYPE},
        http::uri::{Authority, PathAndQuery, Scheme},
    },
};
use lantaran_rules::{MapLocal, MapRemote, content_type_for};

/// The response for a Map Local rule, and a problem to report when the file
/// could not be read. That case gives the device a plain 500, rather than
/// silently reaching the server; the path and the reason stay on this
/// machine, in the returned problem.
pub(crate) async fn local_response(rule: &MapLocal) -> (Response<Body>, Option<String>) {
    match tokio::fs::read(&rule.file).await {
        Ok(bytes) => {
            let mut res = Response::new(Body::from(Full::new(bytes.into())));
            *res.status_mut() = StatusCode::from_u16(rule.status).unwrap_or(StatusCode::OK);
            let content_type = rule
                .content_type
                .clone()
                .unwrap_or_else(|| content_type_for(&rule.file).to_owned());
            if let Ok(value) = content_type.parse() {
                res.headers_mut().insert(CONTENT_TYPE, value);
            }
            res.headers_mut()
                .insert(CACHE_CONTROL, "no-store".parse().expect("valid header"));
            (res, None)
        }
        Err(error) => {
            let mut res =
                Response::new(Body::from("Lantaran Map Local: the file is unavailable\n"));
            *res.status_mut() = StatusCode::INTERNAL_SERVER_ERROR;
            (res, Some(format!("could not read {}: {error}", rule.file)))
        }
    }
}

/// The URL a Map Remote rule sends `uri` to. Parts the rule leaves unset
/// are kept, and the query always is.
pub(crate) fn remote_uri(uri: &Uri, rule: &MapRemote) -> Result<Uri, String> {
    let scheme = match &rule.scheme {
        Some(scheme) => Scheme::try_from(scheme.as_str()).map_err(|e| e.to_string())?,
        None => uri.scheme().cloned().unwrap_or(Scheme::HTTP),
    };

    let host = rule
        .host
        .clone()
        .or_else(|| uri.host().map(str::to_owned))
        .ok_or("the request has no host")?;
    // A new host without a port uses the scheme's default port.
    let port = match (rule.port, &rule.host) {
        (Some(port), _) => Some(port),
        (None, Some(_)) => None,
        (None, None) => uri.port_u16(),
    };
    let host = if host.contains(':') && !host.starts_with('[') {
        format!("[{host}]")
    } else {
        host
    };
    let authority = match port {
        Some(port) => format!("{host}:{port}"),
        None => host,
    };
    let authority = Authority::try_from(authority.as_str()).map_err(|e| e.to_string())?;

    let path = rule.path.clone().unwrap_or_else(|| uri.path().to_owned());
    let path_and_query = match uri.query() {
        Some(query) => format!("{path}?{query}"),
        None => path,
    };
    let path_and_query =
        PathAndQuery::try_from(path_and_query.as_str()).map_err(|e| e.to_string())?;

    Uri::builder()
        .scheme(scheme)
        .authority(authority)
        .path_and_query(path_and_query)
        .build()
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule() -> MapRemote {
        MapRemote {
            pattern: "*".into(),
            method: None,
            scheme: None,
            host: None,
            port: None,
            path: None,
        }
    }

    #[test]
    fn host_and_port_are_replaced_and_the_rest_kept() {
        let uri: Uri = "https://api.example.com/v1/users?page=2".parse().unwrap();
        let mapped = remote_uri(
            &uri,
            &MapRemote {
                scheme: Some("http".into()),
                host: Some("localhost".into()),
                port: Some(3000),
                ..rule()
            },
        )
        .unwrap();
        assert_eq!(mapped.to_string(), "http://localhost:3000/v1/users?page=2");
    }

    #[test]
    fn a_new_host_without_a_port_drops_the_old_port() {
        let uri: Uri = "https://api.example.com:8443/x".parse().unwrap();
        let mapped = remote_uri(
            &uri,
            &MapRemote {
                host: Some("staging.example.com".into()),
                ..rule()
            },
        )
        .unwrap();
        assert_eq!(mapped.to_string(), "https://staging.example.com/x");
    }

    #[test]
    fn only_the_path_can_change() {
        let uri: Uri = "https://api.example.com:8443/v1/a?b=c".parse().unwrap();
        let mapped = remote_uri(
            &uri,
            &MapRemote {
                path: Some("/v2/a".into()),
                ..rule()
            },
        )
        .unwrap();
        assert_eq!(mapped.to_string(), "https://api.example.com:8443/v2/a?b=c");
    }

    #[test]
    fn an_ipv6_host_is_bracketed() {
        let uri: Uri = "http://example.com/".parse().unwrap();
        let mapped = remote_uri(
            &uri,
            &MapRemote {
                host: Some("::1".into()),
                port: Some(8080),
                ..rule()
            },
        )
        .unwrap();
        assert_eq!(mapped.to_string(), "http://[::1]:8080/");
    }

    #[test]
    fn a_bad_host_is_an_error() {
        let uri: Uri = "http://example.com/".parse().unwrap();
        assert!(
            remote_uri(
                &uri,
                &MapRemote {
                    host: Some("bad host".into()),
                    ..rule()
                }
            )
            .is_err()
        );
    }
}
