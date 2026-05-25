use std::collections::{HashMap, HashSet};
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
    download_dir: PathBuf,
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

        let download_dir = PathBuf::from(download_dir);
        let session = Session::new_with_opts(download_dir.clone(), opts)
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
            download_dir,
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

        // Resolve the handle from either variant, rejecting the unused ListOnly path.
        let h = match response {
            AddTorrentResponse::Added(_, h) | AddTorrentResponse::AlreadyManaged(_, h) => h,
            AddTorrentResponse::ListOnly(_) => {
                return Err(EngineError::Backend {
                    reason: "unexpected list_only response".to_string(),
                });
            }
        };
        // Hold the lock across both the ptr_eq lookup and the conditional insert so two
        // concurrent adds (including the case where librqbit returns Added twice for the
        // same magnet) cannot allocate separate IDs for the same handle.
        let (id, handle) = {
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

    pub async fn set_file_selection(
        &self,
        id: u64,
        selected_indexes: Vec<u32>,
    ) -> Result<(), EngineError> {
        if selected_indexes.is_empty() {
            return Ok(());
        }

        let handle = {
            let handles = self.handles.lock().expect("handles lock poisoned");
            handles.get(&id).cloned().ok_or(EngineError::NotFound { id })?
        };

        let only_files: HashSet<usize> =
            selected_indexes.into_iter().map(|i| i as usize).collect();

        self.session
            .update_only_files(&handle, &only_files)
            .await
            .map_err(|e| EngineError::Backend { reason: e.to_string() })?;

        // Remove files that are no longer selected so they don't linger on disk as
        // pre-allocated stubs created during librqbit's initialization phase.
        self.delete_non_selected_files(&handle, &only_files);

        // Unpause unconditionally; if a concurrent set_file_selection call already
        // unpaused the torrent the "already live" error is benign — selection was applied.
        match self.session.unpause(&handle).await {
            Ok(()) => {}
            Err(e) if e.to_string().contains("already live") => {}
            Err(e) => return Err(EngineError::Backend { reason: e.to_string() }),
        }

        Ok(())
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

impl Engine {
    // Deletes pre-allocated stub files for non-selected files after the initial file
    // selection. Guards against destroying real data by checking per-file download
    // progress: a file with any bytes already downloaded is never deleted.
    // Mirrors librqbit's subfolder layout (get_default_subfolder_for_torrent).
    // Errors are never propagated — cleanup is best-effort.
    fn delete_non_selected_files(&self, handle: &TorrentHandle, selected: &HashSet<usize>) {
        // file_progress[i] is the number of bytes verified for file i; 0 means the
        // file contains only pre-allocated space, not real torrent data.
        let file_progress = handle.stats().file_progress;

        let torrent_name = handle.name();
        let paths: Vec<PathBuf> = match handle.with_metadata(|meta| {
            let file_count = meta.file_infos.len();
            // Replicate librqbit's get_default_subfolder_for_torrent logic:
            // multi-file torrents are placed in a named subdirectory.
            let base = if file_count >= 2 {
                if let Some(name) = &torrent_name {
                    // Strip traversal components (e.g. "../..") from the torrent-supplied
                    // name before using it as a directory component.
                    let safe: PathBuf = std::path::Path::new(name.as_str())
                        .components()
                        .filter(|c| matches!(c, std::path::Component::Normal(_)))
                        .collect();
                    if safe.as_os_str().is_empty() {
                        self.download_dir.clone()
                    } else {
                        self.download_dir.join(safe)
                    }
                } else {
                    // Nameless multi-file: librqbit uses the stem of the largest file.
                    let stem = meta
                        .file_infos
                        .iter()
                        .filter(|fi| !fi.attrs.padding)
                        .max_by_key(|fi| fi.len)
                        .and_then(|fi| fi.relative_filename.file_stem())
                        .map(PathBuf::from);
                    stem.map(|s| self.download_dir.join(s))
                        .unwrap_or_else(|| self.download_dir.clone())
                }
            } else {
                self.download_dir.clone()
            };
            meta.file_infos
                .iter()
                .enumerate()
                .filter(|(i, fi)| {
                    !fi.attrs.padding
                        && !selected.contains(i)
                        && file_progress.get(*i).copied().unwrap_or(0) == 0
                })
                .filter_map(|(_, fi)| {
                    // Strip traversal components from the torrent-relative path so a
                    // crafted file entry cannot escape the download directory.
                    let safe_rel: PathBuf = fi
                        .relative_filename
                        .components()
                        .filter(|c| matches!(c, std::path::Component::Normal(_)))
                        .collect();
                    if safe_rel.as_os_str().is_empty() {
                        None
                    } else {
                        Some(base.join(safe_rel))
                    }
                })
                .collect::<Vec<_>>()
        }) {
            Ok(v) => v,
            Err(_) => return,
        };
        for path in paths {
            let _ = std::fs::remove_file(&path);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::time::Duration;

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
            download_dir: dir.to_owned(),
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

    #[tokio::test]
    async fn empty_selection_is_noop() {
        // The empty-vec guard fires before the id lookup, so Ok even for a nonexistent id.
        let dir = TempDir::new().unwrap();
        let engine = make_test_engine(dir.path()).await;
        let result = engine.set_file_selection(999, vec![]).await;
        assert!(result.is_ok(), "empty selection must be a no-op regardless of id");
    }

    #[tokio::test]
    async fn set_file_selection_not_found() {
        let dir = TempDir::new().unwrap();
        let engine = make_test_engine(dir.path()).await;
        let err = engine.set_file_selection(99, vec![0]).await.unwrap_err();
        assert!(matches!(err, EngineError::NotFound { id: 99 }));
    }

    #[tokio::test]
    async fn torrent_files_not_found() {
        let dir = TempDir::new().unwrap();
        let engine = make_test_engine(dir.path()).await;
        let err = engine.torrent_files(42).unwrap_err();
        assert!(matches!(err, EngineError::NotFound { id: 42 }));
    }

    #[tokio::test]
    async fn set_file_selection_not_found_with_duplicate_indexes() {
        // Non-empty duplicate input passes the empty guard and reaches the id lookup.
        let dir = TempDir::new().unwrap();
        let engine = make_test_engine(dir.path()).await;
        let err = engine.set_file_selection(77, vec![0, 0, 0]).await.unwrap_err();
        assert!(matches!(err, EngineError::NotFound { id: 77 }));
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
            download_dir: dir.path().to_owned(),
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

    // Exercises the full file-listing and selection path against a live session.
    // Requires outbound network access (DHT/trackers).
    // Run with: cargo test -- --ignored
    #[tokio::test]
    #[ignore]
    async fn file_selection_live() {
        let dir = TempDir::new().unwrap();
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
            download_dir: dir.path().to_owned(),
            next_id: AtomicU64::new(1),
            handles: Mutex::new(HashMap::new()),
        });

        // Big Buck Bunny — a well-known multi-file torrent used for live tests.
        let magnet =
            "magnet:?xt=urn:btih:dd8255ecdc7ca55fb0bbf81323d87062db1f6d1c&dn=Big+Buck+Bunny";
        let info = engine.add_magnet(magnet.to_string()).await.expect("add_magnet");

        // Wait up to 30 s for metadata resolution (DHT peer discovery can be slow).
        let files = {
            let mut resolved = vec![];
            for _ in 0..60 {
                match engine.torrent_files(info.id) {
                    Ok(f) if !f.is_empty() => {
                        resolved = f;
                        break;
                    }
                    _ => tokio::time::sleep(Duration::from_millis(500)).await,
                }
            }
            resolved
        };
        assert!(!files.is_empty(), "metadata must resolve within 30 s on a live network");

        // Before any selection filter, all files report selected=true.
        assert!(files.iter().all(|f| f.selected), "all files selected before any filter");

        // Select only the first file.
        engine
            .set_file_selection(info.id, vec![0])
            .await
            .expect("set_file_selection");

        // The listing must reflect the new selection state.
        let files_after = engine.torrent_files(info.id).expect("torrent_files after selection");
        assert!(
            files_after.len() > 1,
            "Big Buck Bunny must have >1 file for selection test to be meaningful"
        );
        assert!(files_after[0].selected, "first file must be selected");
        assert!(
            files_after[1..].iter().all(|f| !f.selected),
            "non-selected files must have selected=false after filter is applied"
        );
        // Disk verification (only selected file appears under the download dir) requires
        // waiting for actual piece data to be written, which is out of scope for CI.
    }
}
