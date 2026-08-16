uniffi::setup_scaffolding!();

mod types;
pub use types::{EngineError, FileInfo, TorrentInfo, TorrentState, TorrentStats};

mod announce;
mod engine;
mod logging;
mod storage;
mod tracker_proxy;
pub use engine::Engine;

#[uniffi::export]
pub fn ping() -> String {
    "pong".to_string()
}

/// Absolute path of the rolling log file, so the app can reveal it in Finder — inside the
/// sandbox container it is not a path a user can navigate to by hand. `None` only if the
/// home directory cannot be resolved.
#[uniffi::export]
pub fn log_file_path() -> Option<String> {
    logging::log_file_path().map(|p| p.to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ping_returns_pong() {
        assert_eq!(ping(), "pong");
    }
}
