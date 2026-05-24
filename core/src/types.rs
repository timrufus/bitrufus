#[derive(uniffi::Record, Clone, Debug)]
pub struct TorrentInfo {
    pub id: u64,
    pub info_hash: String,
    pub name: String,
    pub total_bytes: u64,
}

#[derive(uniffi::Error, thiserror::Error, Debug)]
pub enum EngineError {
    #[error("invalid magnet link: {reason}")]
    InvalidMagnet { reason: String },
    #[error("torrent not found")]
    NotFound,
    #[error("io error: {reason}")]
    Io { reason: String },
    #[error("backend error: {reason}")]
    Backend { reason: String },
}
