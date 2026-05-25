use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use directories::ProjectDirs;
use librqbit::{
    AddTorrent, AddTorrentOptions, AddTorrentResponse, Magnet, ManagedTorrent, Session,
    SessionOptions, SessionPersistenceConfig,
};

use crate::types::{EngineError, FileInfo, TorrentInfo};

type TorrentHandle = Arc<ManagedTorrent>;

#[derive(uniffi::Object)]
pub struct Engine {
    session: Arc<Session>,
    next_id: AtomicU64,
    handles: Mutex<HashMap<u64, TorrentHandle>>,
}

#[uniffi::export(async_runtime = "tokio")]
impl Engine {
    #[uniffi::constructor]
    pub async fn new(download_dir: String) -> Result<Arc<Self>, EngineError> {
        let persistence_folder = ProjectDirs::from("com", "BitRufus", "BitRufus")
            .ok_or_else(|| EngineError::Io {
                reason: "cannot determine application data directory".to_string(),
            })?
            .data_dir()
            .to_owned();

        let opts = SessionOptions {
            persistence: Some(SessionPersistenceConfig::Json {
                folder: Some(persistence_folder),
            }),
            ..Default::default()
        };

        let session = Session::new_with_opts(PathBuf::from(download_dir), opts)
            .await
            .map_err(|e| EngineError::Backend {
                reason: e.to_string(),
            })?;

        // Restore previously-persisted torrents. Sort by librqbit's session id so
        // restored engine IDs are stable across restarts (same torrents → same order).
        let mut restored: Vec<(usize, TorrentHandle)> =
            session.with_torrents(|iter| iter.map(|(sid, h)| (sid, h.clone())).collect());
        restored.sort_unstable_by_key(|(sid, _)| *sid);

        let mut map = HashMap::with_capacity(restored.len());
        let mut next_counter = 1u64;
        for (_, h) in restored {
            map.insert(next_counter, h);
            next_counter += 1;
        }

        Ok(Arc::new(Self {
            session,
            next_id: AtomicU64::new(next_counter),
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

        // Capture the display name from dn= before the add consumes parsed.
        let dn = parsed.name.clone();

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
                self.handles.lock().expect("handles lock poisoned").insert(id, h.clone());
                (id, h)
            }
            AddTorrentResponse::AlreadyManaged(_, h) => {
                // Hold the lock across both the lookup and the conditional insert so two
                // concurrent duplicate-add calls cannot both observe missing and allocate
                // separate IDs for the same handle.
                let mut map = self.handles.lock().expect("handles lock poisoned");
                let existing = map.iter().find_map(|(&id, eh)| Arc::ptr_eq(eh, &h).then_some(id));
                let id = if let Some(id) = existing {
                    id
                } else {
                    let id = self.next_id.fetch_add(1, Ordering::Relaxed);
                    map.insert(id, h.clone());
                    id
                };
                (id, h)
            }
            AddTorrentResponse::ListOnly(_) => {
                return Err(EngineError::Backend {
                    reason: "unexpected list_only response".to_string(),
                });
            }
        };

        let stats = handle.stats();
        // For paused magnet-only adds, metadata (name, size) is not resolved until
        // the torrent contacts trackers/DHT. Fall back to the dn= display name.
        let name = handle.name().or(dn).unwrap_or_default();

        Ok(TorrentInfo {
            id,
            info_hash: info_hash_str,
            name,
            total_bytes: stats.total_bytes,
        })
    }

    pub fn torrent_files(&self, id: u64) -> Result<Vec<FileInfo>, EngineError> {
        let handle = {
            let handles = self.handles.lock().expect("handles lock poisoned");
            handles.get(&id).cloned().ok_or(EngineError::NotFound { id })?
        };

        let only_files = handle.only_files();

        handle
            .with_metadata(|metadata| {
                metadata
                    .file_infos
                    .iter()
                    .enumerate()
                    .map(|(idx, fi)| FileInfo {
                        index: idx as u32,
                        path: fi.relative_filename.to_string_lossy().into_owned(),
                        size_bytes: fi.len,
                        selected: only_files.as_ref().is_none_or(|sel| sel.contains(&idx)),
                    })
                    .collect::<Vec<_>>()
            })
            .map_err(|e| EngineError::Backend {
                reason: e.to_string(),
            })
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use librqbit::{Session, SessionOptions};
    use tempfile::TempDir;

    use super::*;

    async fn make_test_engine(dir: &Path) -> Arc<Engine> {
        let session = Session::new_with_opts(
            dir.to_owned(),
            SessionOptions {
                disable_dht: true,
                disable_dht_persistence: true,
                ..Default::default()
            },
        )
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
    async fn handles_map_unchanged_on_parse_failure() {
        let dir = TempDir::new().unwrap();
        let engine = make_test_engine(dir.path()).await;
        let result = engine.add_magnet("garbage://link".to_string()).await;
        assert!(result.is_err(), "expected error for invalid magnet, got {:?}", result);
        assert_eq!(
            engine.handles.lock().expect("lock").len(),
            0,
            "handles map must be empty when magnet parse fails"
        );
    }

    // Verifies ids are strictly increasing across real torrent additions.
    // Run with: cargo test -- --ignored
    #[tokio::test]
    #[ignore]
    async fn id_allocation_is_monotonic_live() {
        let dir = TempDir::new().unwrap();
        // DHT must be enabled so librqbit can accept magnet-only links (no tracker).
        // Disable persistence to avoid writing DHT state between test runs.
        let session = Session::new_with_opts(
            dir.path().to_owned(),
            SessionOptions {
                disable_dht_persistence: true,
                ..Default::default()
            },
        )
        .await
        .expect("session creation");
        let engine = Arc::new(Engine {
            session,
            next_id: AtomicU64::new(1),
            handles: Mutex::new(HashMap::new()),
        });
        let magnet1 =
            "magnet:?xt=urn:btih:dd8255ecdc7ca55fb0bbf81323d87062db1f6d1c&dn=Big+Buck+Bunny";
        let magnet2 =
            "magnet:?xt=urn:btih:08ada5a7a6183aae1e09d831df6748d566095a10&dn=Sintel";
        let info1 = engine.add_magnet(magnet1.to_string()).await.unwrap();
        let info2 = engine.add_magnet(magnet2.to_string()).await.unwrap();
        assert!(info2.id > info1.id, "second add must get a higher id than first (got {} then {})", info1.id, info2.id);
    }
}
