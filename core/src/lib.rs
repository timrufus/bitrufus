uniffi::setup_scaffolding!();

mod types;
pub use types::{EngineError, FileInfo, TorrentInfo, TorrentState, TorrentStats};

mod engine;
pub use engine::Engine;

#[uniffi::export]
pub fn ping() -> String {
    "pong".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ping_returns_pong() {
        assert_eq!(ping(), "pong");
    }
}
