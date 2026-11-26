use crate::HostPattern;

/// Decides which HTTPS hosts the proxy decrypts.
///
/// A host is decrypted when it matches an include pattern and no exclude
/// pattern. Everything else is tunnelled untouched, so an empty list decrypts
/// nothing. That keeps apps with certificate pinning working until the user
/// opts a host in.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SslProxyingList {
    include: Vec<HostPattern>,
    exclude: Vec<HostPattern>,
}

impl SslProxyingList {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn include(mut self, pattern: impl Into<HostPattern>) -> Self {
        self.include.push(pattern.into());
        self
    }

    pub fn exclude(mut self, pattern: impl Into<HostPattern>) -> Self {
        self.exclude.push(pattern.into());
        self
    }

    pub fn included(&self) -> &[HostPattern] {
        &self.include
    }

    pub fn excluded(&self) -> &[HostPattern] {
        &self.exclude
    }

    pub fn should_decrypt(&self, host: &str) -> bool {
        self.include.iter().any(|p| p.matches(host))
            && !self.exclude.iter().any(|p| p.matches(host))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_list_decrypts_nothing() {
        assert!(!SslProxyingList::new().should_decrypt("example.com"));
    }

    #[test]
    fn included_hosts_are_decrypted() {
        let list = SslProxyingList::new()
            .include("api.example.com")
            .include("*.staging.test");
        assert!(list.should_decrypt("api.example.com"));
        assert!(list.should_decrypt("web.staging.test"));
        assert!(!list.should_decrypt("other.example.com"));
    }

    #[test]
    fn exclude_wins_over_include() {
        let list = SslProxyingList::new().include("*").exclude("*.apple.com");
        assert!(list.should_decrypt("api.example.com"));
        assert!(!list.should_decrypt("gateway.icloud.apple.com"));
    }
}
