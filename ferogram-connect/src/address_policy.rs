//! Bounded direct-connect failover with an in-memory address-family preference.
use std::{
    future::Future,
    io,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    sync::atomic::{AtomicU8, Ordering},
    time::Duration,
};

use crate::ConnectError;

const ATTEMPT_TIMEOUT: Duration = Duration::from_secs(4);

#[derive(Clone, Copy)]
#[repr(u8)]
enum Family {
    Unknown = 0,
    Ipv4 = 4,
    Ipv6 = 6,
}

// Shared by fresh connects, migration, media sockets and reconnects. This is
// only a hint: every eligible dial keeps the other family as its fallback.
static ROUTE_PREFERENCE: RoutePreference = RoutePreference::new();

struct RoutePreference(AtomicU8);

impl RoutePreference {
    const fn new() -> Self {
        Self(AtomicU8::new(Family::Unknown as u8))
    }

    fn candidates(&self, addr: &str, dc_id: i16) -> Vec<String> {
        let mut candidates = vec![addr.to_owned()];
        if let Some(fallback) = alternate(addr, dc_id) {
            // No other data is published with this hint, so relaxed ordering
            // suffices. Concurrent dials may finish in either order.
            let preferred = self.0.load(Ordering::Relaxed);
            let fallback_first = preferred == family(&fallback) as u8;
            candidates.push(fallback);
            if fallback_first {
                candidates.swap(0, 1);
            }
        }
        candidates
    }
}

/// Parse numeric endpoints, including unbracketed IPv6 config entries.
pub(crate) fn socket_address(addr: &str) -> Option<SocketAddr> {
    addr.parse::<SocketAddr>().ok().or_else(|| {
        let (host, port) = addr.rsplit_once(':')?;
        Some(SocketAddr::new(host.parse().ok()?, port.parse().ok()?))
    })
}

fn family(addr: &str) -> Family {
    match socket_address(addr).map(|socket| socket.ip()) {
        Some(IpAddr::V4(_)) => Family::Ipv4,
        Some(IpAddr::V6(_)) => Family::Ipv6,
        None => Family::Unknown,
    }
}

fn endpoints(dc_id: i16) -> Option<(&'static str, &'static str)> {
    match dc_id {
        1 => Some(("149.154.175.53", "2001:b28:f23d:f001::a")),
        2 => Some(("149.154.167.51", "2001:67c:4e8:f002::a")),
        3 => Some(("149.154.175.100", "2001:b28:f23d:f003::a")),
        4 => Some(("149.154.167.91", "2001:67c:4e8:f004::a")),
        5 => Some(("91.108.56.130", "2001:b28:f23f:f005::a")),
        _ => None,
    }
}

fn telegram_ipv4(ip: Ipv4Addr) -> bool {
    let [a, b, c, _] = ip.octets();
    (a == 149 && b == 154 && (160..176).contains(&c))
        || (a == 91 && b == 108 && matches!(c / 4, 1 | 2 | 3 | 4 | 5 | 14))
}

/// Only production Telegram IPs on port 443 qualify. Hostname/custom/test/CDN
/// routes and proxies retain their original endpoint and semantics.
pub(crate) fn alternate(addr: &str, dc_id: i16) -> Option<String> {
    let (v4, v6) = endpoints(dc_id)?;
    let socket = socket_address(addr)?;
    if socket.port() != 443 {
        return None;
    }
    let other: IpAddr = match socket.ip() {
        IpAddr::V4(ip) if telegram_ipv4(ip) => v6.parse().ok()?,
        IpAddr::V6(ip) if ip == v6.parse::<std::net::Ipv6Addr>().ok()? => v4.parse().ok()?,
        _ => return None,
    };
    Some(SocketAddr::new(other, 443).to_string())
}

/// Try each address at most once, without detached tasks or RPC/login replay.
/// A protocol/auth rejection is not a network failure and must propagate.
pub(crate) async fn connect<T, F, Fut>(addr: &str, dc_id: i16, dial: F) -> Result<T, ConnectError>
where
    F: FnMut(String) -> Fut,
    Fut: Future<Output = Result<T, ConnectError>>,
{
    connect_with_preference(addr, dc_id, &ROUTE_PREFERENCE, dial).await
}

async fn connect_with_preference<T, F, Fut>(
    addr: &str,
    dc_id: i16,
    preference: &RoutePreference,
    mut dial: F,
) -> Result<T, ConnectError>
where
    F: FnMut(String) -> Fut,
    Fut: Future<Output = Result<T, ConnectError>>,
{
    let candidates = preference.candidates(addr, dc_id);
    let eligible = candidates.len() == 2;
    let mut failures = Vec::new();
    for candidate in candidates {
        let result = tokio::time::timeout(ATTEMPT_TIMEOUT, dial(candidate.clone()))
            .await
            .unwrap_or_else(|_| {
                Err(ConnectError::Io(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "TCP/MTProto reachability timed out",
                )))
            });
        match result {
            Ok(value) => {
                // The production dial includes a nonce-matched MTProto probe,
                // not merely TCP acceptance or a login result. Failed attempts
                // never teach a preference; successful fallback can reverse it.
                if eligible {
                    preference
                        .0
                        .store(family(&candidate) as u8, Ordering::Relaxed);
                }
                tracing::info!(dc_id, endpoint = %candidate, "[ferogram::connect] route ready");
                return Ok(value);
            }
            Err(ConnectError::Io(error)) => {
                tracing::warn!(dc_id, endpoint = %candidate, %error,
                    "[ferogram::connect] route failed");
                failures.push(format!("{candidate}: {error}"));
            }
            Err(error) => return Err(error),
        }
    }
    Err(ConnectError::Io(io::Error::new(
        io::ErrorKind::NotConnected,
        format!("DC{dc_id} direct routes failed: {}", failures.join("; ")),
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    // Single-dial tests must not share the production global hint.
    async fn connect<T, F, Fut>(addr: &str, dc_id: i16, dial: F) -> Result<T, ConnectError>
    where
        F: FnMut(String) -> Fut,
        Fut: Future<Output = Result<T, ConnectError>>,
    {
        connect_with_preference(addr, dc_id, &RoutePreference::new(), dial).await
    }

    fn run(future: impl Future<Output = ()>) {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(future);
    }

    #[test]
    fn successful_fallback_is_reused_across_connections_and_data_centers() {
        run(async {
            let preference = RoutePreference::new();
            let mut calls = Vec::new();
            let value = connect_with_preference("149.154.167.51:443", 2, &preference, |addr| {
                calls.push(addr.clone());
                async move {
                    if addr.starts_with('[') {
                        Ok(42)
                    } else {
                        Err(ConnectError::Io(io::ErrorKind::ConnectionReset.into()))
                    }
                }
            })
            .await
            .unwrap();
            assert_eq!(value, 42);
            assert_eq!(calls, ["149.154.167.51:443", "[2001:67c:4e8:f002::a]:443"]);

            for dc in [2, 1, 3, 4, 5] {
                calls.clear();
                let (v4, v6) = endpoints(dc).unwrap();
                connect_with_preference(&format!("{v4}:443"), dc, &preference, |addr| {
                    calls.push(addr);
                    async { Ok(()) }
                })
                .await
                .unwrap();
                assert_eq!(calls, [format!("[{v6}]:443")]);
            }
        });
    }

    #[test]
    fn ipv6_failure_can_relearn_ipv4_without_rewriting_the_supplied_endpoint() {
        run(async {
            let preference = RoutePreference::new();
            connect_with_preference("2001:b28:f23f:f005::a:443", 5, &preference, |_| async {
                Ok(())
            })
            .await
            .unwrap();

            let mut calls = Vec::new();
            connect_with_preference("91.108.56.156:443", 5, &preference, |addr| {
                calls.push(addr.clone());
                async move {
                    if addr.starts_with('[') {
                        Err(ConnectError::Io(io::ErrorKind::ConnectionReset.into()))
                    } else {
                        Ok(())
                    }
                }
            })
            .await
            .unwrap();
            assert_eq!(calls, ["[2001:b28:f23f:f005::a]:443", "91.108.56.156:443"]);
            assert_eq!(preference.0.load(Ordering::Relaxed), Family::Ipv4 as u8);
            assert_eq!(
                preference.candidates("91.108.56.156:443", 5)[0],
                "91.108.56.156:443"
            );
            for addr in ["[2001:67c:4e8:f002::a]:443", "2001:67c:4e8:f002::a:443"] {
                assert_eq!(preference.candidates(addr, 2)[0], "149.154.167.51:443");
            }
        });
    }

    #[test]
    fn unknown_preference_honors_the_supplied_family() {
        for addr in [
            "91.108.56.156:443",
            "[2001:b28:f23f:f005::a]:443",
            "2001:b28:f23f:f005::a:443",
        ] {
            let preference = RoutePreference::new();
            assert_eq!(preference.candidates(addr, 5)[0], addr);
            run(async {
                connect_with_preference(addr, 5, &preference, |_| async { Ok(()) })
                    .await
                    .unwrap();
            });
            assert_eq!(preference.0.load(Ordering::Relaxed), family(addr) as u8);
        }
    }

    #[test]
    fn failures_and_protocol_rejections_do_not_change_preference() {
        run(async {
            for initial in [Family::Unknown, Family::Ipv4, Family::Ipv6] {
                let preference = RoutePreference(AtomicU8::new(initial as u8));
                let mut calls = Vec::new();
                let result = connect_with_preference::<(), _, _>(
                    "91.108.56.156:443",
                    5,
                    &preference,
                    |addr| {
                        calls.push(addr);
                        async { Err(ConnectError::Io(io::ErrorKind::ConnectionReset.into())) }
                    },
                )
                .await;
                assert!(matches!(result, Err(ConnectError::Io(_))));
                assert_eq!(calls.len(), 2);
                assert_ne!(calls[0], calls[1]);
                assert_eq!(preference.0.load(Ordering::Relaxed), initial as u8);

                calls.clear();
                let result = connect_with_preference::<(), _, _>(
                    "91.108.56.156:443",
                    5,
                    &preference,
                    |addr| {
                        calls.push(addr);
                        async { Err(ConnectError::TransportCode(-404)) }
                    },
                )
                .await;
                assert!(matches!(result, Err(ConnectError::TransportCode(-404))));
                assert_eq!(calls.len(), 1);
                assert_eq!(preference.0.load(Ordering::Relaxed), initial as u8);
            }
        });
    }

    #[test]
    fn excluded_routes_neither_follow_nor_teach_the_global_preference() {
        run(async {
            for initial in [Family::Unknown, Family::Ipv4, Family::Ipv6] {
                let preference = RoutePreference(AtomicU8::new(initial as u8));
                for (addr, dc) in [
                    ("127.0.0.1:443", 2),
                    ("custom.example:443", 2),
                    ("149.154.167.51:80", 2),
                    ("149.154.167.51:443", 202),
                    ("149.154.167.51:443", -2),
                    ("[::1]:443", 2),
                ] {
                    let mut calls = Vec::new();
                    connect_with_preference(addr, dc, &preference, |candidate| {
                        calls.push(candidate);
                        async { Ok(()) }
                    })
                    .await
                    .unwrap();
                    assert_eq!(calls, [addr]);
                    assert_eq!(preference.0.load(Ordering::Relaxed), initial as u8);
                }
            }
        });
    }

    #[test]
    fn stalled_preferred_ipv6_has_a_deadline_and_relearns_ipv4() {
        run(async {
            let preference = RoutePreference(AtomicU8::new(Family::Ipv6 as u8));
            let result =
                connect_with_preference("91.108.56.156:443", 5, &preference, |addr| async move {
                    if addr.starts_with('[') {
                        std::future::pending::<()>().await;
                    }
                    Ok(())
                });
            tokio::time::timeout(ATTEMPT_TIMEOUT + Duration::from_secs(1), result)
                .await
                .unwrap()
                .unwrap();
            assert_eq!(preference.0.load(Ordering::Relaxed), Family::Ipv4 as u8);
        });
    }

    #[test]
    #[ignore = "uses real Telegram endpoints; no phone, credentials or disk sessions"]
    fn live_production_routes() {
        use crate::{Connection, TransportKind};
        run(async {
            for (dc, addr) in [(2, "149.154.167.51:443"), (5, "91.108.56.156:443")] {
                for transport in [
                    TransportKind::Abridged,
                    TransportKind::Obfuscated { secret: None },
                ] {
                    let conn = Connection::connect_raw(addr, None, None, &transport, dc)
                        .await
                        .unwrap();
                    eprintln!(
                        "DC{dc} requested {addr}, connected {} with {transport:?}",
                        conn.stream.peer_addr().unwrap()
                    );
                    let key = conn.auth_key_bytes();
                    let salt = conn.enc.salt;
                    let offset = conn.enc.time_offset;
                    drop(conn);
                    let conn = Connection::connect_with_key(
                        addr, key, salt, offset, None, None, &transport, dc, false,
                    )
                    .await
                    .unwrap();
                    assert_eq!(conn.auth_key_bytes(), key);
                    eprintln!(
                        "DC{dc} cached key preserved, connected {}",
                        conn.stream.peer_addr().unwrap()
                    );
                    drop(conn);
                    let (stream, _) = Connection::open_stream_pub(addr, dc, &transport, None, None)
                        .await
                        .unwrap();
                    eprintln!(
                        "DC{dc} sender/pool reconnect route {}",
                        stream.peer_addr().unwrap()
                    );
                }
            }
        });
    }

    #[test]
    fn stalled_primary_has_a_deadline_then_uses_alternate() {
        run(async {
            let result = connect("91.108.56.156:443", 5, |addr| async move {
                if !addr.starts_with('[') {
                    std::future::pending::<()>().await;
                }
                Ok(())
            });
            tokio::time::timeout(ATTEMPT_TIMEOUT + Duration::from_secs(1), result)
                .await
                .unwrap()
                .unwrap();
        });
    }

    #[test]
    fn migration_uses_target_dc_not_bootstrap_dc() {
        assert_eq!(
            alternate("91.108.56.156:443", 5).as_deref(),
            Some("[2001:b28:f23f:f005::a]:443")
        );
        assert_eq!(
            alternate("149.154.167.51:443", 2).as_deref(),
            Some("[2001:67c:4e8:f002::a]:443")
        );
    }

    #[test]
    fn ipv6_sessions_can_fall_back_to_ipv4() {
        for addr in ["[2001:b28:f23f:f005::a]:443", "2001:b28:f23f:f005::a:443"] {
            assert_eq!(alternate(addr, 5).as_deref(), Some("91.108.56.130:443"));
        }
    }

    #[test]
    fn custom_test_and_cdn_endpoints_are_not_remapped() {
        for (addr, dc) in [
            ("127.0.0.1:443", 2),
            ("custom.example:443", 2),
            ("149.154.167.51:80", 2),
            ("149.154.167.51:443", 202),
            ("149.154.167.51:443", -2),
            ("[::1]:443", 2),
        ] {
            assert!(alternate(addr, dc).is_none());
        }
    }

    #[test]
    fn reset_falls_back_once_and_returns_success() {
        run(async {
            let mut calls = Vec::new();
            let result = connect("91.108.56.156:443", 5, |addr| {
                calls.push(addr.clone());
                async move {
                    if addr.starts_with('[') {
                        Ok(42)
                    } else {
                        Err(ConnectError::Io(io::ErrorKind::ConnectionReset.into()))
                    }
                }
            })
            .await
            .unwrap();
            assert_eq!(result, 42);
            assert_eq!(calls.len(), 2);
        });
    }

    #[test]
    fn two_failures_stop_and_keep_both_errors() {
        run(async {
            let mut calls = 0;
            let error = connect::<(), _, _>("91.108.56.156:443", 5, |_| {
                calls += 1;
                async { Err(ConnectError::Io(io::ErrorKind::ConnectionReset.into())) }
            })
            .await
            .unwrap_err();
            assert_eq!(calls, 2);
            let message = error.to_string();
            assert!(message.contains("91.108.56.156"));
            assert!(message.contains("2001:b28:f23f:f005::a"));
        });
    }

    #[test]
    fn success_and_auth_errors_never_trigger_second_attempt() {
        run(async {
            let mut calls = 0;
            connect("91.108.56.156:443", 5, |_| {
                calls += 1;
                async { Ok(()) }
            })
            .await
            .unwrap();
            assert_eq!(calls, 1);
            let mut calls = 0;
            let result = connect::<(), _, _>("91.108.56.156:443", 5, |_| {
                calls += 1;
                async { Err(ConnectError::TransportCode(-404)) }
            })
            .await;
            assert!(matches!(result, Err(ConnectError::TransportCode(-404))));
            assert_eq!(calls, 1);
        });
    }
}
