//! Whether the launcher's own server is up, shown before the player
//! connects: its health, `https://<host>/tpf3mp/health`, asked once a
//! minute. The host's reverse proxy passes that one path to the server's
//! `/healthz`, which answers `ok` (deploy/nginx.conf.example); anything
//! else, such as the proxy's 502 while the server is down, is offline. The
//! request opens no session on the server and carries nothing of the
//! player's.

use std::{
    sync::{
        Arc,
        atomic::{AtomicU8, Ordering},
    },
    time::Duration,
};

/// How often the server is asked.
const EVERY: Duration = Duration::from_secs(60);
/// How long an answer may take.
const PATIENCE: Duration = Duration::from_secs(8);

/// What the last request found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reach {
    /// Not asked yet.
    Unknown,
    Online,
    Offline,
}

/// The server's reach, kept up to date on a thread of its own, which ends
/// once the probe is dropped.
#[derive(Debug)]
pub struct Probe {
    reach: Arc<AtomicU8>,
    /// The server asked; `None` for one that asks nothing.
    server: Option<String>,
}

impl Probe {
    /// Starts asking `server`, a `host:port`; `None` when it is not one.
    pub fn start(server: &str) -> Option<Self> {
        let url = probe_url(server)?;
        let reach = Arc::new(AtomicU8::new(Reach::Unknown as u8));
        // Weak: the thread stops asking once the probe is gone, as when the
        // player changed the server.
        let shared = Arc::downgrade(&reach);
        std::thread::Builder::new()
            .name("server-probe".into())
            .spawn(move || {
                let agent = ureq::Agent::new_with_config(
                    ureq::Agent::config_builder()
                        .https_only(true)
                        .http_status_as_error(false)
                        .timeout_global(Some(PATIENCE))
                        .build(),
                );
                while shared.strong_count() > 0 {
                    let answer = agent
                        .get(&url)
                        .header(
                            "User-Agent",
                            concat!("tpf3mp-launcher/", env!("CARGO_PKG_VERSION")),
                        )
                        .call()
                        .map(|response| response.status().as_u16());
                    match shared.upgrade() {
                        Some(reach) => reach.store(classify(answer.ok()) as u8, Ordering::Relaxed),
                        None => break,
                    }
                    std::thread::sleep(EVERY);
                }
            })
            .ok()?;
        Some(Self {
            reach,
            server: Some(server.trim().to_owned()),
        })
    }

    /// One that always shows `reach`, asking nothing: for tests and
    /// screenshots.
    pub fn showing(reach: Reach) -> Self {
        Self {
            reach: Arc::new(AtomicU8::new(reach as u8)),
            server: None,
        }
    }

    /// Whether this probe tells of `server`: one that asks nothing tells of
    /// any.
    pub fn tells_of(&self, server: &str) -> bool {
        self.server
            .as_deref()
            .is_none_or(|asked| asked.eq_ignore_ascii_case(server.trim()))
    }

    pub fn reach(&self) -> Reach {
        match self.reach.load(Ordering::Relaxed) {
            value if value == Reach::Online as u8 => Reach::Online,
            value if value == Reach::Offline as u8 => Reach::Offline,
            _ => Reach::Unknown,
        }
    }
}

/// The health address for `host:port`, which the server answers on
/// through the host's reverse proxy.
pub fn probe_url(server: &str) -> Option<String> {
    let (host, port) = server.trim().rsplit_once(':')?;
    port.parse::<u16>().ok()?;
    let plain = host.trim_start_matches('[').trim_end_matches(']');
    let valid = !plain.is_empty()
        && plain
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | ':'));
    valid.then(|| format!("https://{host}/tpf3mp/health"))
}

/// An answer's status, or none at all: only the server's own "ok" is up.
fn classify(status: Option<u16>) -> Reach {
    match status {
        Some(200) => Reach::Online,
        _ => Reach::Offline,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_server_is_asked_for_its_health() {
        assert_eq!(
            probe_url("tpf3mp.213-133-98-90.sslip.io:29470").as_deref(),
            Some("https://tpf3mp.213-133-98-90.sslip.io/tpf3mp/health")
        );
        assert_eq!(
            probe_url("[2001:db8::1]:29470").as_deref(),
            Some("https://[2001:db8::1]/tpf3mp/health")
        );
        for bad in ["", "no-port", "host:port", "evil.example/x:29470", ":29470"] {
            assert_eq!(probe_url(bad), None, "{bad}");
        }
    }

    #[test]
    fn a_probe_tells_of_its_own_server() {
        let shown = Probe::showing(Reach::Online);
        assert!(
            shown.tells_of("any.example:1"),
            "one for tests tells of any"
        );
        let asking = Probe {
            reach: Arc::new(AtomicU8::new(Reach::Unknown as u8)),
            server: Some("eu.example:29470".into()),
        };
        assert!(asking.tells_of(" EU.example:29470"));
        assert!(!asking.tells_of("us.example:29470"));
    }

    #[test]
    fn only_the_servers_ok_is_online() {
        assert_eq!(classify(Some(200)), Reach::Online);
        assert_eq!(classify(None), Reach::Offline, "no answer");
        // The proxy with the server down, or a host without the path.
        for status in [404, 500, 502, 503, 504] {
            assert_eq!(classify(Some(status)), Reach::Offline);
        }
    }
}
