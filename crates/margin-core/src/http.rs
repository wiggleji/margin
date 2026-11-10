//! Shared HTTP agent: blocking `ureq` (no async runtime → small binary).
//! TLS uses the OS trust store (rustls-platform-verifier), so corporate
//! TLS-inspection CAs installed in the macOS Keychain just work, and
//! `HTTPS_PROXY` / `NO_PROXY` are honoured.

use std::time::Duration;

pub(crate) fn agent() -> ureq::Agent {
    let tls = ureq::tls::TlsConfig::builder()
        .root_certs(ureq::tls::RootCerts::PlatformVerifier)
        .build();
    ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(20)))
        .tls_config(tls)
        .user_agent("margin-git-client")
        .build()
        .into()
}
