use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use directories::ProjectDirs;
use librqbit::{
    AddTorrent, AddTorrentOptions, AddTorrentResponse, Magnet, ManagedTorrent, Session,
    SessionOptions, SessionPersistenceConfig,
};

use crate::types::{EngineError, TorrentInfo};

type TorrentHandle = Arc<ManagedTorrent>;

#[derive(uniffi::Object)]
#[allow(dead_code)]
pub struct Engine {
    pub(crate) session: Arc<Session>,
    pub(crate) next_id: AtomicU64,
    pub(crate) handles: Mutex<HashMap<u64, TorrentHandle>>,
}

#[uniffi::export]
impl Engine {
    #[uniffi::constructor]
    pub async fn new(download_dir: String) -> Result<Arc<Self>, EngineError> {
        let persistence_folder = ProjectDirs::from("com", "BitRufus", "BitRufus")
            .map(|dirs| dirs.data_dir().to_owned());

        let opts = SessionOptions {
            persistence: Some(SessionPersistenceConfig::Json {
                folder: persistence_folder,
            }),
            ..Default::default()
        };

        let session = Session::new_with_opts(PathBuf::from(download_dir), opts)
            .await
            .map_err(|e| EngineError::Backend {
                reason: e.to_string(),
            })?;

        let restored: Vec<TorrentHandle> = session
            .with_torrents(|iter| iter.map(|(_sid, h)| h.clone()).collect());
        let next = restored.len() as u64 + 1;
        let map: HashMap<u64, TorrentHandle> = (1u64..).zip(restored).collect();

        Ok(Arc::new(Self {
            session,
            next_id: AtomicU64::new(next),
            handles: Mutex::new(map),
        }))
    }

    pub async fn add_magnet(&self, magnet: String) -> Result<TorrentInfo, EngineError> {
        let parsed = Magnet::parse(&magnet).map_err(|e| EngineError::InvalidMagnet {
            reason: e.to_string(),
        })?;

        let info_hash_str = parsed
            .as_id20()
            .ok_or_else(|| EngineError::InvalidMagnet {
                reason: "magnet link missing BTv1 infohash".to_string(),
            })?
            .as_string();

        let response = self
            .session
            .add_torrent(
                AddTorrent::from_url(&magnet),
                Some(AddTorrentOptions {
                    paused: true,
                    ..Default::default()
                }),
            )
            .await
            .map_err(|e| EngineError::Backend {
                reason: e.to_string(),
            })?;

        let (id, handle) = match response {
            AddTorrentResponse::Added(_, h) => {
                let id = self.next_id.fetch_add(1, Ordering::Relaxed);
                self.handles.lock().unwrap().insert(id, h.clone());
                (id, h)
            }
            AddTorrentResponse::AlreadyManaged(_, h) => {
                let mut map = self.handles.lock().unwrap();
                let existing_id = map
                    .iter()
                    .find(|(_, eh)| eh.info_hash() == h.info_hash())
                    .map(|(id, _)| *id);
                let id = match existing_id {
                    Some(id) => id,
                    None => {
                        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
                        map.insert(id, h.clone());
                        id
                    }
                };
                (id, h)
            }
            AddTorrentResponse::ListOnly(_) => unreachable!("list_only not set"),
        };

        let stats = handle.stats();
        let name = handle.name().unwrap_or_default();

        Ok(TorrentInfo {
            id,
            info_hash: info_hash_str,
            name,
            total_bytes: stats.total_bytes,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::sync::atomic::Ordering;

    use librqbit::{Session, SessionOptions};
    use tempfile::TempDir;

    use super::*;

    async fn make_test_engine(dir: &Path) -> Arc<Engine> {
        let session = Session::new_with_opts(dir.to_owned(), SessionOptions::default())
            .await
            .expect("session creation");
        Arc::new(Engine {
            session,
            next_id: AtomicU64::new(1),
            handles: Mutex::new(HashMap::new()),
        })
    }

    #[tokio::test]
    async fn invalid_magnet_returns_invalid_magnet_error() {
        let dir = TempDir::new().unwrap();
        let engine = make_test_engine(dir.path()).await;
        let err = engine
            .add_magnet("not-a-valid-magnet".to_string())
            .await
            .unwrap_err();
        assert!(matches!(err, EngineError::InvalidMagnet { .. }));
    }

    #[tokio::test]
    async fn id_counter_unchanged_on_parse_failure() {
        let dir = TempDir::new().unwrap();
        let engine = make_test_engine(dir.path()).await;
        let before = engine.next_id.load(Ordering::Relaxed);
        let _ = engine.add_magnet("garbage://link".to_string()).await;
        assert_eq!(
            engine.next_id.load(Ordering::Relaxed),
            before,
            "counter must not advance when magnet parse fails"
        );
    }

    // Verifies ids are strictly increasing across real torrent additions.
    // Run with: cargo test -- --ignored
    #[tokio::test]
    #[ignore]
    async fn id_allocation_is_monotonic_live() {
        let dir = TempDir::new().unwrap();
        let engine = make_test_engine(dir.path()).await;
        let magnet =
            "magnet:?xt=urn:btih:dd8255ecdc7ca55fb0bbf81323d87062db1f6d1c&dn=Big+Buck+Bunny";
        let info = engine.add_magnet(magnet.to_string()).await.unwrap();
        let counter_after = engine.next_id.load(Ordering::Relaxed);
        assert!(
            counter_after > info.id,
            "next_id must be past the allocated id"
        );
    }
}
