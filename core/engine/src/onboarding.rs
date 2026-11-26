//! The page a device opens to get the root certificate.
//!
//! A phone that is already pointed at the proxy can open the proxy's own
//! address, or `http://lantaran.ssl/` through the proxy, and download the
//! certificate from there. The page is plain HTTP on purpose: the device does
//! not trust the certificate yet, so HTTPS to the proxy cannot work.

use bytes::Bytes;
use hudsucker::{
    Body,
    hyper::{
        Response, StatusCode,
        header::{CACHE_CONTROL, CONTENT_DISPOSITION, CONTENT_TYPE},
    },
};

/// The host name that reaches this page through the proxy.
pub const ONBOARDING_HOST: &str = "lantaran.ssl";

/// The certificate in the forms the page offers.
pub(crate) struct CertificateFiles {
    pub(crate) pem: Bytes,
    pub(crate) der: Bytes,
}

/// Which instructions to show.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Platform {
    Ios,
    Android,
    Other,
}

impl Platform {
    fn from_user_agent(user_agent: &str) -> Self {
        if ["iPhone", "iPad", "iPod"]
            .iter()
            .any(|device| user_agent.contains(device))
        {
            Self::Ios
        } else if user_agent.contains("Android") {
            Self::Android
        } else {
            Self::Other
        }
    }
}

/// Answers a request for the onboarding page or one of its downloads.
pub(crate) fn respond(path: &str, user_agent: &str, files: &CertificateFiles) -> Response<Body> {
    match path {
        "/" | "/index.html" => page(Platform::from_user_agent(user_agent)),
        // Android's installer offers .crt files and wants DER or PEM; iOS
        // Safari offers to install a profile for this content type.
        "/lantaran-ca.crt" => download(
            files.der.clone(),
            "application/x-x509-ca-cert",
            "lantaran-ca.crt",
        ),
        "/lantaran-ca.pem" => download(
            files.pem.clone(),
            "application/x-x509-ca-cert",
            "lantaran-ca.pem",
        ),
        _ => {
            let mut res = Response::new(Body::from("Not found\n"));
            *res.status_mut() = StatusCode::NOT_FOUND;
            res
        }
    }
}

fn download(bytes: Bytes, content_type: &'static str, file_name: &str) -> Response<Body> {
    let mut res = Response::new(Body::from(http_body_util::Full::new(bytes)));
    let headers = res.headers_mut();
    headers.insert(CONTENT_TYPE, content_type.parse().expect("valid header"));
    headers.insert(
        CONTENT_DISPOSITION,
        format!("attachment; filename=\"{file_name}\"")
            .parse()
            .expect("valid header"),
    );
    headers.insert(CACHE_CONTROL, "no-store".parse().expect("valid header"));
    res
}

fn page(platform: Platform) -> Response<Body> {
    let (title, steps) = match platform {
        Platform::Ios => ("Set up this iPhone or iPad", IOS_STEPS),
        Platform::Android => ("Set up this Android device", ANDROID_STEPS),
        Platform::Other => ("Set up this device", OTHER_STEPS),
    };
    let html = PAGE.replace("{title}", title).replace("{steps}", steps);

    let mut res = Response::new(Body::from(html));
    res.headers_mut().insert(
        CONTENT_TYPE,
        "text/html; charset=utf-8".parse().expect("valid header"),
    );
    res.headers_mut()
        .insert(CACHE_CONTROL, "no-store".parse().expect("valid header"));
    res
}

const PAGE: &str = r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Lantaran</title>
<style>
  body { font: 16px/1.5 -apple-system, system-ui, sans-serif; margin: 0; padding: 24px;
         max-width: 560px; color: #1d1d1f; background: #fafafa; }
  @media (prefers-color-scheme: dark) { body { color: #f5f5f7; background: #1c1c1e; } }
  h1 { font-size: 22px; margin: 0 0 4px; }
  p.lead { margin: 0 0 20px; opacity: .75; }
  a.button { display: block; text-align: center; padding: 14px; border-radius: 12px;
             background: #0a84ff; color: #fff; text-decoration: none; font-weight: 600;
             margin: 16px 0 24px; }
  ol { padding-left: 20px; } li { margin: 8px 0; }
  .note { font-size: 14px; opacity: .75; border-top: 1px solid rgba(127,127,127,.3);
          padding-top: 12px; margin-top: 24px; }
  code { font-size: 14px; }
</style>
</head>
<body>
<h1>{title}</h1>
<p class="lead">Install the Lantaran root certificate so this device's HTTPS traffic can be inspected.</p>
<a class="button" href="/lantaran-ca.crt">Download the certificate</a>
{steps}
<p class="note">This certificate lets the computer running Lantaran read this device's encrypted
traffic to the hosts you choose. Remove it when you are done debugging.</p>
</body>
</html>
"#;

const IOS_STEPS: &str = r#"<ol>
  <li>Tap <b>Download the certificate</b>, then <b>Allow</b>. Use Safari; other browsers cannot install it.</li>
  <li>Open <b>Settings</b>, tap <b>Profile Downloaded</b> near the top, then <b>Install</b>.</li>
  <li>Go to <b>Settings &gt; General &gt; About &gt; Certificate Trust Settings</b> and switch on
      <b>Lantaran Root CA</b>. Without this step HTTPS will not be readable.</li>
</ol>"#;

const ANDROID_STEPS: &str = r#"<ol>
  <li>Tap <b>Download the certificate</b>. It is saved as <code>lantaran-ca.crt</code>.</li>
  <li>Open <b>Settings &gt; Security &gt; Encryption &amp; credentials &gt; Install a certificate &gt;
      CA certificate</b> (the names differ between manufacturers; search Settings for
      "CA certificate"), then pick the downloaded file.</li>
  <li>Browsers will now be readable. Most apps ignore certificates installed this way: an app
      you build can opt in for debug builds with a <code>network_security_config</code> that
      trusts <code>user</code> certificates.</li>
</ol>"#;

const OTHER_STEPS: &str = r#"<ol>
  <li>Download the certificate and add it to this system's trusted root certificates.</li>
  <li>On iOS and Android, open this page on the device itself for specific steps.</li>
</ol>"#;

#[cfg(test)]
mod tests {
    use super::*;

    const IPHONE: &str =
        "Mozilla/5.0 (iPhone; CPU iPhone OS 18_1 like Mac OS X) AppleWebKit/605.1.15";
    const PIXEL: &str = "Mozilla/5.0 (Linux; Android 15; Pixel 8) AppleWebKit/537.36";

    #[test]
    fn platform_is_read_from_the_user_agent() {
        assert_eq!(Platform::from_user_agent(IPHONE), Platform::Ios);
        assert_eq!(Platform::from_user_agent(PIXEL), Platform::Android);
        assert_eq!(Platform::from_user_agent("curl/8.7.1"), Platform::Other);
    }
}
