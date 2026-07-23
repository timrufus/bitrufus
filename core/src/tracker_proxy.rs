//! Local HTTP announce proxy: the missing "re-announce with a proper User-Agent" path.
//!
//! `announce.rs` fixes tracker announces at *add* time via `initial_peers`, but those
//! are a one-shot snapshot that librqbit does not persist: after an app restart (or on
//! a long download) a torrent is back to librqbit's own UA-less announces, which some
//! trackers (rutracker's bt*.t-ru.org) reject with 403. librqbit 8.1.1 exposes no API
//! to inject peers into a live torrent, so instead we run a tiny announce server on
//! 127.0.0.1 and register it as a *session-level tracker*. librqbit then announces to
//! it through its normal TrackerComms machinery — on add, resume, restore, and
//! periodically per the interval we return — and the proxy forwards each announce to
//! the torrent's real HTTP trackers (proper UA, real listen port, bounded timeout),
//! merging the returned peers back in compact form.
//!
//! Scope notes:
//! - librqbit merges session trackers only into non-private torrents, so private
//!   torrents never hit the proxy (their own passkey trackers are announced directly
//!   by librqbit, same as before).
//! - The proxy binds 127.0.0.1 only; the worst a local caller can do is trigger an
//!   outbound announce for a torrent that is already in the session.

use std::net::SocketAddr;
use std::sync::{Arc, OnceLock, Weak};
use std::time::Duration;

use librqbit::dht::Id20;
use librqbit::{api::TorrentIdOrHash, Session};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use crate::announce::{self, AnnounceParams};

// Re-announce cadence we ask librqbit for. Short enough to keep the swarm fresh on
// long downloads, long enough not to hammer upstream trackers (rutracker asks ~59 min).
const INTERVAL_WITH_PEERS: u64 = 900;
// When we have nothing useful to return (session not ready, unknown torrent, upstream
// gave zero peers), ask librqbit to come back sooner.
const INTERVAL_EMPTY: u64 = 300;
const MAX_REQUEST_BYTES: usize = 8 * 1024;
const CONNECTION_TIMEOUT: Duration = Duration::from_secs(30);

pub(crate) struct TrackerProxy {
    port: u16,
    inner: Arc<ProxyInner>,
    accept_task: tokio::task::JoinHandle<()>,
}

struct ProxyInner {
    port: u16,
    // Set once right after the librqbit session is created (the proxy must exist
    // *before* the session so its URL can go into SessionOptions::trackers). Weak so
    // the proxy never keeps a dropped engine's session alive.
    session: OnceLock<Weak<Session>>,
}

impl TrackerProxy {
    pub(crate) async fn start() -> anyhow::Result<Self> {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
        let port = listener.local_addr()?.port();
        let inner = Arc::new(ProxyInner {
            port,
            session: OnceLock::new(),
        });
        let accept_task = tokio::spawn(accept_loop(listener, inner.clone()));
        tracing::info!(port, "tracker announce proxy listening on 127.0.0.1");
        Ok(Self {
            port,
            inner,
            accept_task,
        })
    }

    pub(crate) fn announce_url(&self) -> String {
        format!("http://127.0.0.1:{}/announce", self.port)
    }

    pub(crate) fn set_session(&self, session: &Arc<Session>) {
        let _ = self.inner.session.set(Arc::downgrade(session));
    }
}

impl Drop for TrackerProxy {
    fn drop(&mut self) {
        self.accept_task.abort();
    }
}

async fn accept_loop(listener: TcpListener, inner: Arc<ProxyInner>) {
    loop {
        match listener.accept().await {
            Ok((stream, _)) => {
                let inner = inner.clone();
                tokio::spawn(async move {
                    let _ =
                        tokio::time::timeout(CONNECTION_TIMEOUT, handle_connection(stream, inner))
                            .await;
                });
            }
            Err(e) => {
                tracing::warn!(error = ?e, "tracker proxy accept error");
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }
    }
}

async fn handle_connection(mut stream: TcpStream, inner: Arc<ProxyInner>) -> anyhow::Result<()> {
    let mut buf = Vec::with_capacity(1024);
    let mut tmp = [0u8; 1024];
    while !buf.windows(4).any(|w| w == b"\r\n\r\n") {
        if buf.len() > MAX_REQUEST_BYTES {
            anyhow::bail!("request too large");
        }
        let n = stream.read(&mut tmp).await?;
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&tmp[..n]);
    }

    let body = match request_query(&buf) {
        Some(query) => build_announce_response(query, &inner).await,
        None => bencode_failure("bad request"),
    };
    let header = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(header.as_bytes()).await?;
    stream.write_all(&body).await?;
    stream.flush().await?;
    Ok(())
}

// Extracts the query string from "GET /announce?<query> HTTP/1.1".
fn request_query(request: &[u8]) -> Option<&str> {
    let line_end = request.windows(2).position(|w| w == b"\r\n")?;
    let line = std::str::from_utf8(&request[..line_end]).ok()?;
    let mut parts = line.split(' ');
    if parts.next()? != "GET" {
        return None;
    }
    let path = parts.next()?;
    let (_, query) = path.split_once('?')?;
    Some(query)
}

async fn build_announce_response(query: &str, inner: &ProxyInner) -> Vec<u8> {
    let mut info_hash: Option<[u8; 20]> = None;
    let mut peer_id: Option<[u8; 20]> = None;
    let mut port: u16 = 0;
    let mut uploaded: u64 = 0;
    let mut downloaded: u64 = 0;
    let mut left: u64 = 1;
    let mut event: Option<String> = None;

    for pair in query.split('&') {
        let Some((key, value)) = pair.split_once('=') else {
            continue;
        };
        match key {
            "info_hash" => info_hash = to_20_bytes(&percent_decode(value)),
            "peer_id" => peer_id = to_20_bytes(&percent_decode(value)),
            "port" => port = value.parse().unwrap_or(0),
            "uploaded" => uploaded = value.parse().unwrap_or(0),
            "downloaded" => downloaded = value.parse().unwrap_or(0),
            "left" => left = value.parse().unwrap_or(1),
            "event" => event = Some(value.to_string()),
            _ => {}
        }
    }

    let Some(info_hash) = info_hash else {
        return bencode_failure("missing info_hash");
    };
    let Some(session) = inner.session.get().and_then(|w| w.upgrade()) else {
        // Session not wired up yet (early startup) — tell librqbit to retry soon.
        return bencode_peers(INTERVAL_EMPTY, &[]);
    };
    let Some(handle) = session.get(TorrentIdOrHash::Hash(Id20::new(info_hash))) else {
        return bencode_peers(INTERVAL_EMPTY, &[]);
    };

    // The torrent's own trackers only (session trackers are merged by librqbit at
    // announce time, not stored on the handle) — but filter defensively so the proxy
    // can never announce to itself.
    let trackers: Vec<String> = handle
        .shared()
        .trackers
        .iter()
        .filter(|u| !is_self(u, inner.port))
        .map(|u| u.to_string())
        .collect();
    if trackers.is_empty() {
        return bencode_peers(INTERVAL_EMPTY, &[]);
    }

    // While a torrent is paused/resolving librqbit announces port=0; substitute the
    // session's real listen port so upstream trackers register a usable peer entry.
    if port == 0 {
        port = session.tcp_listen_port().unwrap_or(0);
    }

    let params = AnnounceParams {
        info_hash,
        peer_id: peer_id.unwrap_or([b'0'; 20]),
        port,
        uploaded,
        downloaded,
        left,
        event,
    };

    let peers = announce::announce_all(&trackers, &params).await;

    tracing::debug!(
        info_hash = ?Id20::new(info_hash),
        trackers = trackers.len(),
        peers = peers.len(),
        "proxied announce"
    );

    let interval = if peers.is_empty() {
        INTERVAL_EMPTY
    } else {
        INTERVAL_WITH_PEERS
    };

    bencode_peers(interval, &peers)
}

fn is_self(url: &url::Url, own_port: u16) -> bool {
    matches!(url.host_str(), Some("127.0.0.1") | Some("localhost")) && url.port() == Some(own_port)
}

fn percent_decode(s: &str) -> Vec<u8> {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(b) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(b);
                i += 3;
                continue;
            }
        }
        // '+' means space in form encoding; trackers don't use it in binary fields,
        // but decode it correctly anyway.
        out.push(if bytes[i] == b'+' { b' ' } else { bytes[i] });
        i += 1;
    }
    out
}

fn to_20_bytes(v: &[u8]) -> Option<[u8; 20]> {
    let mut out = [0u8; 20];
    if v.len() != 20 {
        return None;
    }
    out.copy_from_slice(v);
    Some(out)
}

// Bencoded announce response in the shape librqbit's TrackerResponse expects:
// complete/incomplete/interval are required fields, peers/peers6 in compact form.
// Keys are emitted in lexicographic order per the bencode spec.
fn bencode_peers(interval: u64, peers: &[SocketAddr]) -> Vec<u8> {
    let mut v4 = Vec::new();
    let mut v6 = Vec::new();
    for peer in peers {
        match peer {
            SocketAddr::V4(a) => {
                v4.extend_from_slice(&a.ip().octets());
                v4.extend_from_slice(&a.port().to_be_bytes());
            }
            SocketAddr::V6(a) => {
                v6.extend_from_slice(&a.ip().octets());
                v6.extend_from_slice(&a.port().to_be_bytes());
            }
        }
    }
    let mut out = Vec::with_capacity(64 + v4.len() + v6.len());
    out.extend_from_slice(b"d8:completei0e10:incompletei0e8:intervali");
    out.extend_from_slice(interval.to_string().as_bytes());
    out.extend_from_slice(b"e5:peers");
    out.extend_from_slice(v4.len().to_string().as_bytes());
    out.push(b':');
    out.extend_from_slice(&v4);
    if !v6.is_empty() {
        out.extend_from_slice(b"6:peers6");
        out.extend_from_slice(v6.len().to_string().as_bytes());
        out.push(b':');
        out.extend_from_slice(&v6);
    }
    out.push(b'e');
    out
}

fn bencode_failure(reason: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(32 + reason.len());
    out.extend_from_slice(b"d14:failure reason");
    out.extend_from_slice(reason.len().to_string().as_bytes());
    out.push(b':');
    out.extend_from_slice(reason.as_bytes());
    out.push(b'e');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::announce::parse_compact_peers;

    #[test]
    fn request_query_extracts_announce_query() {
        let req = b"GET /announce?info_hash=%01&port=51413 HTTP/1.1\r\nHost: x\r\n\r\n";
        assert_eq!(request_query(req), Some("info_hash=%01&port=51413"));
        assert_eq!(request_query(b"POST /announce?x=1 HTTP/1.1\r\n\r\n"), None);
        assert_eq!(request_query(b"GET /announce HTTP/1.1\r\n\r\n"), None);
    }

    #[test]
    fn percent_decode_binary_roundtrip() {
        assert_eq!(
            percent_decode("%1b%ed%f2A+B"),
            vec![0x1b, 0xed, 0xf2, b'A', b' ', b'B']
        );
        assert_eq!(percent_decode("abc"), b"abc".to_vec());
        // Truncated escape at the end must not panic.
        assert_eq!(percent_decode("ab%2"), b"ab%2".to_vec());
    }

    #[test]
    fn bencode_peers_roundtrips_through_compact_parser() {
        let peers: Vec<SocketAddr> = vec![
            "1.2.3.4:6881".parse().unwrap(),
            "[2001:db8::1]:51413".parse().unwrap(),
        ];
        let body = bencode_peers(900, &peers);
        let parsed = parse_compact_peers(&body);
        assert_eq!(parsed, peers);
        // Interval must be present in librqbit's expected shape.
        let s = String::from_utf8_lossy(&body);
        assert!(s.starts_with("d8:completei0e10:incompletei0e8:intervali900e5:peers"));
    }

    #[test]
    fn self_url_is_excluded() {
        assert!(is_self(
            &url::Url::parse("http://127.0.0.1:9999/announce").unwrap(),
            9999
        ));
        assert!(is_self(
            &url::Url::parse("http://localhost:9999/announce").unwrap(),
            9999
        ));
        assert!(!is_self(
            &url::Url::parse("http://127.0.0.1:1234/announce").unwrap(),
            9999
        ));
        assert!(!is_self(
            &url::Url::parse("http://bt3.t-ru.org/ann").unwrap(),
            9999
        ));
    }

    // End-to-end over a real localhost socket: proxy with no session wired must answer
    // a valid bencoded empty-peers response with the short retry interval.
    #[tokio::test]
    async fn proxy_answers_empty_when_session_missing() {
        let proxy = TrackerProxy::start().await.expect("proxy start");
        let url = proxy.announce_url();
        let addr = url
            .strip_prefix("http://")
            .unwrap()
            .split('/')
            .next()
            .unwrap()
            .to_string();

        let mut stream = TcpStream::connect(&addr).await.expect("connect");
        let ih = "%01".repeat(20);
        let req = format!(
            "GET /announce?info_hash={ih}&peer_id={ih}&port=0&uploaded=0&downloaded=0&left=1&compact=1 HTTP/1.1\r\nHost: {addr}\r\n\r\n"
        );
        stream.write_all(req.as_bytes()).await.unwrap();
        let mut response = Vec::new();
        stream.read_to_end(&mut response).await.unwrap();

        let response = String::from_utf8_lossy(&response);
        assert!(response.starts_with("HTTP/1.1 200 OK"), "got: {response}");
        assert!(
            response.contains(&format!(
                "d8:completei0e10:incompletei0e8:intervali{INTERVAL_EMPTY}e5:peers0:e"
            )),
            "got: {response}"
        );
    }
}
