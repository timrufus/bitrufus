#[derive(uniffi::Record, Clone, Debug)]
pub struct FileInfo {
    pub index: u32,
    pub path: String,
    pub size_bytes: u64,
    pub selected: bool,
}

#[derive(uniffi::Record, Clone, Debug)]
pub struct TorrentInfo {
    pub id: u64,
    pub info_hash: String,
    pub name: String,
    pub total_bytes: u64,
}

// Maps from librqbit's TorrentStatsState (exposed via ManagedTorrent::stats()):
//
//   librqbit TorrentStatsState   │  TorrentState
//   ─────────────────────────────┼──────────────
//   Initializing                 │  Initializing
//   Paused                       │  Paused
//   Live  (stats.finished=false) │  Downloading
//   Live  (stats.finished=true)  │  Seeding
//   Error                        │  Error
//
// The exhaustive `match` in Engine::torrent_stats (engine.rs) has no wildcard arm, so
// adding a new TorrentStatsState variant in a future librqbit upgrade causes a compile
// error caught by `cargo build` / `cargo test`.
#[derive(uniffi::Enum, Clone, Debug, PartialEq)]
pub enum TorrentState {
    Paused,
    Initializing,
    Downloading,
    Seeding,
    Error,
}

#[derive(uniffi::Record, Clone, Debug)]
pub struct TorrentStats {
    pub id: u64,
    pub state: TorrentState,
    pub downloaded_bytes: u64,
    pub total_bytes: u64,
    pub download_speed_bps: u64,
    pub upload_speed_bps: u64,
    pub peer_count: u32,
}

#[derive(uniffi::Error, thiserror::Error, Debug)]
pub enum EngineError {
    #[error("invalid magnet link: {reason}")]
    InvalidMagnet { reason: String },
    #[error("torrent not found: {id}")]
    NotFound { id: u64 },
    #[error("io error: {reason}")]
    Io { reason: String },
    #[error("backend error: {reason}")]
    Backend { reason: String },
}
