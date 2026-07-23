//! Manual HTTP tracker announce used to gather initial peers for an add.
//!
//! librqbit's own HTTP announces are rejected by some trackers: its reqwest client
//! sends no User-Agent header, and e.g. rutracker's bt*.t-ru.org (behind an
//! anti-bot proxy) answers 403 to every UA-less request. librqbit 8.1.1 offers no
//! way to configure the UA, and while a torrent is added paused it additionally
//! announces `port=0`. So we announce ourselves — proper User-Agent, real listen
//! port, bounded timeout — and feed the resulting peers to
//! `AddTorrentOptions::initial_peers`. librqbit stores those in the torrent's
//! options and re-merges them into the peer stream on every unpause.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::time::Duration;

// Per-tracker bound; announces run concurrently so this also bounds the whole step.
const ANNOUNCE_TIMEOUT: Duration = Duration::from_secs(6);
const USER_AGENT: &str = concat!("BitRufus/", env!("CARGO_PKG_VERSION"));

/// Announces to every HTTP(S) tracker concurrently and returns the deduplicated
/// union of returned peers. Best-effort: any tracker error just contributes no
/// peers. UDP trackers are skipped — librqbit's own UDP tracker client handles
/// those fine (no User-Agent involved).
pub(crate) async fn gather_initial_peers(
    trackers: &[String],
    info_hash: [u8; 20],
    listen_port: u16,
) -> Vec<SocketAddr> {
    let client = match reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .timeout(ANNOUNCE_TIMEOUT)
        .build()
    {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };

    let mut tasks = Vec::new();
    for tracker in trackers {
        if !(tracker.starts_with("http://") || tracker.starts_with("https://")) {
            continue;
        }
        let url = announce_url(tracker, &info_hash, listen_port);
        let client = client.clone();
        tasks.push(tokio::spawn(async move {
            let response = client.get(&url).send().await.ok()?;
            let body = response.bytes().await.ok()?;
            Some(parse_compact_peers(&body))
        }));
    }

    let mut peers: Vec<SocketAddr> = Vec::new();
    for task in tasks {
        if let Ok(Some(mut p)) = task.await {
            peers.append(&mut p);
        }
    }
    peers.sort();
    peers.dedup();
    if !peers.is_empty() {
        tracing::info!(count = peers.len(), "gathered initial peers from HTTP trackers");
    }
    peers
}

fn announce_url(tracker: &str, info_hash: &[u8; 20], listen_port: u16) -> String {
    use std::fmt::Write;
    let mut encoded_hash = String::with_capacity(60);
    for b in info_hash {
        let _ = write!(encoded_hash, "%{b:02x}");
    }
    let separator = if tracker.contains('?') { '&' } else { '?' };
    // left=1 (not 0): a zero "left" marks us as a seeder, which makes some
    // trackers return fewer or zero peers.
    format!(
        "{tracker}{separator}info_hash={encoded_hash}&peer_id={}&port={listen_port}\
         &uploaded=0&downloaded=0&left=1&event=started&compact=1",
        peer_id()
    )
}

// Azureus-style peer id: fixed client prefix + 12 URL-safe chars. Uniqueness only
// needs to be good enough for tracker bookkeeping, so clock nanos suffice.
fn peer_id() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("-BR0100-{:012x}", nanos & 0xffff_ffff_ffff)
}

// Extracts peers from a bencoded tracker response in compact form (BEP 23 "peers"
// as 6-byte chunks, BEP 7 "peers6" as 18-byte chunks). compact=1 is requested and
// honored by effectively all trackers; a non-compact (list-of-dicts) response
// simply fails the length parse below and yields no peers.
fn parse_compact_peers(body: &[u8]) -> Vec<SocketAddr> {
    let mut out = Vec::new();
    if let Some(v4) = find_bencode_bytes(body, b"5:peers") {
        for c in v4.chunks_exact(6) {
            let ip = Ipv4Addr::new(c[0], c[1], c[2], c[3]);
            let port = u16::from_be_bytes([c[4], c[5]]);
            if port != 0 {
                out.push(SocketAddr::new(IpAddr::V4(ip), port));
            }
        }
    }
    if let Some(v6) = find_bencode_bytes(body, b"6:peers6") {
        for c in v6.chunks_exact(18) {
            let mut octets = [0u8; 16];
            octets.copy_from_slice(&c[..16]);
            let port = u16::from_be_bytes([c[16], c[17]]);
            if port != 0 {
                out.push(SocketAddr::new(IpAddr::V6(Ipv6Addr::from(octets)), port));
            }
        }
    }
    out
}

// Finds the bencoded `key` and returns the byte-string value following it
// (`<len>:<bytes>`). Not a general bencode parser: tracker responses are tiny
// flat dicts, and the exact `N:key` byte pattern cannot collide with string
// content except pathologically — in which case the parse fails closed (None).
fn find_bencode_bytes<'a>(buf: &'a [u8], key: &[u8]) -> Option<&'a [u8]> {
    let pos = buf.windows(key.len()).position(|w| w == key)? + key.len();
    let rest = &buf[pos..];
    let colon = rest.iter().position(|&b| b == b':')?;
    let len: usize = std::str::from_utf8(&rest[..colon]).ok()?.parse().ok()?;
    rest.get(colon + 1..colon + 1 + len)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_compact_v4_peers() {
        // d8:intervali1800e5:peers12:<two 6-byte peers>e
        let mut body = b"d8:intervali1800e5:peers12:".to_vec();
        body.extend_from_slice(&[1, 2, 3, 4, 0x1A, 0xE1]); // 1.2.3.4:6881
        body.extend_from_slice(&[5, 6, 7, 8, 0x00, 0x50]); // 5.6.7.8:80
        body.extend_from_slice(b"e");
        let peers = parse_compact_peers(&body);
        assert_eq!(
            peers,
            vec![
                "1.2.3.4:6881".parse::<SocketAddr>().unwrap(),
                "5.6.7.8:80".parse::<SocketAddr>().unwrap(),
            ]
        );
    }

    #[test]
    fn skips_zero_ports_and_truncated_chunks() {
        let mut body = b"d5:peers11:".to_vec();
        body.extend_from_slice(&[1, 2, 3, 4, 0, 0]); // port 0 — dropped
        body.extend_from_slice(&[9, 9, 9, 9, 9]); // truncated chunk — dropped
        body.extend_from_slice(b"e");
        assert!(parse_compact_peers(&body).is_empty());
    }

    #[test]
    fn non_compact_peer_list_yields_nothing() {
        // "peers" as a bencoded list (non-compact) must fail the length parse.
        let body = b"d5:peersld2:ip7:1.2.3.44:porti6881eeee";
        assert!(parse_compact_peers(body).is_empty());
    }

    #[test]
    fn failure_response_yields_nothing() {
        let body = b"d14:failure reason13:not authorizede";
        assert!(parse_compact_peers(body).is_empty());
    }

    #[test]
    fn announce_url_shape() {
        let url = announce_url("http://tr.example/ann", &[0xAB; 20], 6881);
        assert!(url.starts_with("http://tr.example/ann?info_hash=%ab%ab"));
        assert!(url.contains("&port=6881&"));
        assert!(url.contains("&left=1&"));
        assert!(url.contains("&compact=1"));
        assert!(url.contains("peer_id=-BR0100-"));
    }
}
