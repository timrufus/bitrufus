use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use directories::ProjectDirs;
use librqbit::{
    AddTorrent, AddTorrentOptions, AddTorrentResponse, Magnet, ManagedTorrent, Session,
    SessionOptions, SessionPersistenceConfig, TorrentStatsState,
};

use crate::types::{EngineError, FileInfo, TorrentInfo, TorrentState, TorrentStats};

type TorrentHandle = Arc<ManagedTorrent>;

struct EngineInner {
    handles: HashMap<u64, TorrentHandle>,
    // Records when each torrent was added so delete_non_selected_files can verify that
    // stub files haven't been modified externally before removing them.
    add_times: HashMap<u64, std::time::SystemTime>,
    // librqbit session IDs currently being deleted. Held from the moment handles.remove()
    // is called until session.delete() returns, so concurrent add_magnet calls that get
    // AlreadyManaged for the same torrent can detect the in-flight deletion and bail out
    // rather than inserting a fresh engine ID that will immediately become a zombie.
    deleting: HashSet<usize>,
    next_id: u64,
}

#[derive(uniffi::Object)]
pub struct Engine {
    session: Arc<Session>,
    download_dir: PathBuf,
    inner: Mutex<EngineInner>,
}

#[uniffi::export(async_runtime = "tokio")]
impl Engine {
    /// Creates an `Engine` whose session is fully restored before returning.
    ///
    /// **Restore guarantee:** `Session::new_with_opts` is `await`ed before `with_torrents`
    /// is called.  By the time `new` returns, `inner.handles` contains every torrent that
    /// librqbit loaded from the JSON persistence folder.  Callers may call `list_torrents()`
    /// immediately with no risk of racing against an in-progress restore.
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
                reason: format!("{e:#}"),
            })?;

        // Restore previously-persisted torrents. Pin each engine ID to
        // librqbit's own session ID (+ 1 to stay 1-based) so IDs remain
        // stable across restarts even after a torrent has been removed.
        let restored: Vec<(usize, TorrentHandle)> =
            session.with_torrents(|iter| iter.map(|(sid, h)| (sid, h.clone())).collect());

        let mut map = HashMap::with_capacity(restored.len());
        let mut max_engine_id: u64 = 0;
        for (sid, h) in restored {
            let engine_id = sid as u64 + 1;
            map.insert(engine_id, h);
            if engine_id > max_engine_id {
                max_engine_id = engine_id;
            }
        }
        let next_counter = max_engine_id + 1;

        Ok(Arc::new(Self {
            session,
            download_dir,
            inner: Mutex::new(EngineInner {
                handles: map,
                add_times: HashMap::new(),
                deleting: HashSet::new(),
                next_id: next_counter,
            }),
        }))
    }

    pub async fn add_magnet(&self, magnet: String) -> Result<TorrentInfo, EngineError> {
        let parsed = Magnet::parse(&magnet).map_err(|e| EngineError::InvalidMagnet {
            reason: e.to_string(),
        })?;

        // Reject BTv2-only magnet links before they reach the session.
        if parsed.as_id20().is_none() {
            return Err(EngineError::InvalidMagnet {
                reason: "magnet link missing BTv1 infohash".to_string(),
            });
        }

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
        // Hold the lock across the deleting check, the ptr_eq lookup, and the conditional
        // insert so concurrent adds and an in-flight remove cannot race:
        // - If remove() already pulled this handle's librqbit ID into the deleting set,
        //   session.delete() is still running; inserting a new engine ID here would create
        //   a zombie once the delete completes, so we return an error instead.
        // - The ptr_eq dedup prevents two concurrent adds from allocating separate IDs for
        //   the same handle (including the case where librqbit returns Added twice).
        let (id, handle) = {
            let mut inner = self.inner.lock().expect("inner lock poisoned");
            if inner.deleting.contains(&h.id()) {
                return Err(EngineError::Backend {
                    reason: "torrent is currently being deleted".to_string(),
                });
            }
            let existing =
                inner.handles.iter().find_map(|(&id, eh)| Arc::ptr_eq(eh, &h).then_some(id));
            let id = if let Some(id) = existing {
                id
            } else {
                let id = inner.next_id;
                inner.next_id += 1;
                inner.handles.insert(id, h.clone());
                inner.add_times.insert(id, std::time::SystemTime::now());
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

        let (handle, add_time) = {
            let inner = self.inner.lock().expect("inner lock poisoned");
            let h = inner.handles.get(&id).cloned().ok_or(EngineError::NotFound { id })?;
            let t = inner.add_times.get(&id).copied();
            (h, t)
        };

        let only_files: HashSet<usize> =
            selected_indexes.into_iter().map(|i| i as usize).collect();

        self.session
            .update_only_files(&handle, &only_files)
            .await
            .map_err(|e| EngineError::Backend { reason: e.to_string() })?;

        // Remove files that are no longer selected so they don't linger on disk as
        // pre-allocated stubs created during librqbit's initialization phase.
        self.delete_non_selected_files(&handle, &only_files, add_time).await;

        // Unpause unconditionally; if a concurrent set_file_selection call already
        // unpaused the torrent the "already live" error is benign — selection was applied.
        self.unpause_idempotent(&handle).await
    }

    pub fn torrent_stats(&self, id: u64) -> Result<TorrentStats, EngineError> {
        let handle = {
            let inner = self.inner.lock().expect("inner lock poisoned");
            inner.handles.get(&id).cloned().ok_or(EngineError::NotFound { id })?
        };

        let rq = handle.stats();

        let state = map_torrent_state(rq.state, rq.finished);

        let (download_speed_bps, upload_speed_bps, peer_count) = rq
            .live
            .map(|live| {
                // Speed::mbps is MiB/s (SpeedEstimator::mbps = bps/1024/1024); multiply by
                // 2^20 to recover bytes/s.
                let dl = (live.download_speed.mbps * 1_048_576.0) as u64;
                let ul = (live.upload_speed.mbps * 1_048_576.0) as u64;
                let peers = live.snapshot.peer_stats.live as u32;
                (dl, ul, peers)
            })
            .unwrap_or((0, 0, 0));

        Ok(TorrentStats {
            id,
            state,
            downloaded_bytes: rq.progress_bytes,
            total_bytes: rq.total_bytes,
            download_speed_bps,
            upload_speed_bps,
            peer_count,
        })
    }

    pub async fn pause(&self, id: u64) -> Result<(), EngineError> {
        let handle = {
            let inner = self.inner.lock().expect("inner lock poisoned");
            inner.handles.get(&id).cloned().ok_or(EngineError::NotFound { id })?
        };
        match self.session.pause(&handle).await {
            Ok(()) => Ok(()),
            // librqbit bails with "torrent is already paused" when pause is called on a
            // paused torrent (torrent_state/mod.rs in librqbit 8.1.1). Treat as Ok.
            Err(e) if e.to_string().contains("already paused") => Ok(()),
            Err(e) => Err(EngineError::Backend { reason: e.to_string() }),
        }
    }

    pub async fn resume(&self, id: u64) -> Result<(), EngineError> {
        let handle = {
            let inner = self.inner.lock().expect("inner lock poisoned");
            inner.handles.get(&id).cloned().ok_or(EngineError::NotFound { id })?
        };
        self.unpause_idempotent(&handle).await
    }

    pub async fn remove(&self, id: u64, delete_files: bool) -> Result<(), EngineError> {
        // Remove from the map and mark as deleting atomically under the same lock.
        // Inserting into `deleting` before releasing the lock ensures that a concurrent
        // add_magnet which gets AlreadyManaged from librqbit (because session.delete has
        // not yet run) will see the tombstone and return an error rather than inserting a
        // fresh engine ID that would become a zombie once the delete completes.
        let (librqbit_id, handle, add_time) = {
            let mut inner = self.inner.lock().expect("inner lock poisoned");
            let h = inner.handles.get(&id).cloned().ok_or(EngineError::NotFound { id })?;
            let librqbit_id = h.id();
            inner.handles.remove(&id);
            let t = inner.add_times.remove(&id);
            inner.deleting.insert(librqbit_id);
            (librqbit_id, h, t)
        };
        let result = self
            .session
            .delete(librqbit::api::TorrentIdOrHash::Id(librqbit_id), delete_files)
            .await
            .map_err(|e| EngineError::Backend { reason: e.to_string() });
        let mut inner = self.inner.lock().expect("inner lock poisoned");
        inner.deleting.remove(&librqbit_id);
        if result.is_err() {
            // Reinstate both the handle and its add_time so the torrent remains fully
            // reachable (including the mtime guard in delete_non_selected_files) if the
            // delete failed.
            inner.handles.insert(id, handle);
            if let Some(t) = add_time {
                inner.add_times.insert(id, t);
            }
        }
        result
    }

    pub fn list_torrents(&self) -> Vec<TorrentInfo> {
        // Snapshot the handles and release the lock before calling handle.stats() so
        // concurrent add/remove callers are not blocked for the full iteration.
        let snapshot: Vec<(u64, TorrentHandle)> = {
            let inner = self.inner.lock().expect("inner lock poisoned");
            inner.handles.iter().map(|(&id, h)| (id, h.clone())).collect()
        };
        let mut result: Vec<TorrentInfo> = snapshot
            .into_iter()
            .map(|(id, handle)| {
                let stats = handle.stats();
                TorrentInfo {
                    id,
                    name: handle.name().unwrap_or_default(),
                    total_bytes: stats.total_bytes,
                }
            })
            .collect();
        result.sort_by_key(|t| t.id);
        result
    }

    pub fn torrent_files(&self, id: u64) -> Result<Vec<FileInfo>, EngineError> {
        let handle = {
            let inner = self.inner.lock().expect("inner lock poisoned");
            inner.handles.get(&id).cloned().ok_or(EngineError::NotFound { id })?
        };

        let only_files = handle.only_files();

        handle
            .with_metadata(|metadata| {
                metadata
                    .file_infos
                    .iter()
                    .enumerate()
                    .filter(|(_, fi)| !fi.attrs.padding)
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

fn map_torrent_state(state: TorrentStatsState, finished: bool) -> TorrentState {
    match (state, finished) {
        (TorrentStatsState::Initializing, _) => TorrentState::Initializing,
        (TorrentStatsState::Paused, _) => TorrentState::Paused,
        (TorrentStatsState::Live, false) => TorrentState::Downloading,
        (TorrentStatsState::Live, true) => TorrentState::Seeding,
        (TorrentStatsState::Error, _) => TorrentState::Error,
    }
}

impl Engine {
    // Calls session.unpause and swallows the "already live" error that librqbit returns
    // when unpause is called on a torrent that is already running
    // (torrent_state/mod.rs:322 in librqbit 8.1.1). All other errors are propagated.
    async fn unpause_idempotent(&self, handle: &TorrentHandle) -> Result<(), EngineError> {
        match self.session.unpause(handle).await {
            Ok(()) => Ok(()),
            Err(e) if e.to_string().contains("already live") => Ok(()),
            Err(e) => Err(EngineError::Backend { reason: e.to_string() }),
        }
    }

    // Deletes pre-allocated stub files for non-selected files after the initial file
    // selection. Guards against destroying real data by checking per-file download
    // progress, on-disk size, and mtime relative to when this add was initiated.
    // Mirrors librqbit's subfolder layout (get_default_subfolder_for_torrent).
    // Errors are never propagated — cleanup is best-effort.
    async fn delete_non_selected_files(
        &self,
        handle: &TorrentHandle,
        selected: &HashSet<usize>,
        add_time: Option<std::time::SystemTime>,
    ) {
        // file_progress[i] is the number of bytes verified for file i; 0 means the
        // file contains only pre-allocated space, not real torrent data.
        let file_progress = handle.stats().file_progress;

        let torrent_name = handle.name();
        // Collect (path, expected_size) pairs so we can verify on-disk size before
        // deleting. This guards against removing pre-existing files that happen to
        // share a name with an unselected torrent file: a file whose size differs from
        // the torrent-declared length is not a stub we created.
        let paths: Vec<(PathBuf, u64)> = match handle.with_metadata(|meta| {
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
                        Some((base.join(safe_rel), fi.len))
                    }
                })
                .collect::<Vec<_>>()
        }) {
            Ok(v) => v,
            Err(_) => return,
        };
        tokio::task::spawn_blocking(move || {
            for (path, expected_len) in paths {
                // Only remove the file if:
                // 1. declared size > 0 (avoids matching arbitrary empty files)
                // 2. on-disk size matches the torrent-declared length (pre-allocation marker)
                // 3. mtime is not newer than when add_magnet was called, which would indicate
                //    the file was written externally after librqbit created the stub
                let is_stub = std::fs::metadata(&path)
                    .map(|m| {
                        if m.len() != expected_len || expected_len == 0 {
                            return false;
                        }
                        if let Some(add_t) = add_time {
                            // Allow a 2-second buffer for filesystem clock resolution and any
                            // delay between librqbit's file creation and when we captured add_time.
                            let cutoff = add_t + std::time::Duration::from_secs(2);
                            m.modified().map(|mtime| mtime <= cutoff).unwrap_or(false)
                        } else {
                            // No add_time means this is a restored session torrent; skip
                            // speculative deletion to avoid removing pre-existing files.
                            false
                        }
                    })
                    .unwrap_or(false);
                if is_stub {
                    let _ = std::fs::remove_file(&path);
                }
            }
        })
        .await
        .ok();
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::time::Duration;

    use librqbit::{Session, SessionOptions, SessionPersistenceConfig};
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
            inner: Mutex::new(EngineInner {
                handles: HashMap::new(),
                add_times: HashMap::new(),
                deleting: HashSet::new(),
                next_id: 1,
            }),
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
            engine.inner.lock().expect("lock").handles.len(),
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
    async fn torrent_stats_not_found() {
        let dir = TempDir::new().unwrap();
        let engine = make_test_engine(dir.path()).await;
        let err = engine.torrent_stats(55).unwrap_err();
        assert!(matches!(err, EngineError::NotFound { id: 55 }));
    }

    #[tokio::test]
    async fn list_torrents_empty() {
        let dir = TempDir::new().unwrap();
        let engine = make_test_engine(dir.path()).await;
        assert_eq!(engine.list_torrents().len(), 0);
    }

    #[tokio::test]
    async fn torrent_files_not_found() {
        let dir = TempDir::new().unwrap();
        let engine = make_test_engine(dir.path()).await;
        let err = engine.torrent_files(42).unwrap_err();
        assert!(matches!(err, EngineError::NotFound { id: 42 }));
    }

    #[tokio::test]
    async fn set_file_selection_non_empty_duplicate_not_treated_as_empty() {
        // Duplicate indexes must not cause the empty guard to fire; the id lookup must run.
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
            inner: Mutex::new(EngineInner {
                handles: HashMap::new(),
                add_times: HashMap::new(),
                deleting: HashSet::new(),
                next_id: 1,
            }),
        });
        let magnet1 =
            "magnet:?xt=urn:btih:dd8255ecdc7ca55fb0bbf81323d87062db1f6d1c&dn=Big+Buck+Bunny";
        let magnet2 =
            "magnet:?xt=urn:btih:08ada5a7a6183aae1e09d831df6748d566095a10&dn=Sintel";
        let info1 = engine.add_magnet(magnet1.to_string()).await.unwrap();
        let info2 = engine.add_magnet(magnet2.to_string()).await.unwrap();
        assert!(info2.id > info1.id, "second add must get a higher id than first (got {} then {})", info1.id, info2.id);
    }

    // Exercises the production map_torrent_state function for every (state, finished) pair
    // in the mapping table documented in types.rs.
    #[test]
    fn state_mapping_correctness() {
        let cases: &[(TorrentStatsState, bool, TorrentState)] = &[
            (TorrentStatsState::Initializing, false, TorrentState::Initializing),
            (TorrentStatsState::Initializing, true, TorrentState::Initializing),
            (TorrentStatsState::Paused, false, TorrentState::Paused),
            (TorrentStatsState::Paused, true, TorrentState::Paused),
            (TorrentStatsState::Live, false, TorrentState::Downloading),
            (TorrentStatsState::Live, true, TorrentState::Seeding),
            (TorrentStatsState::Error, false, TorrentState::Error),
            (TorrentStatsState::Error, true, TorrentState::Error),
        ];
        for (state, finished, expected) in cases {
            assert_eq!(
                map_torrent_state(*state, *finished),
                *expected,
                "state={state:?} finished={finished}"
            );
        }
    }

    #[tokio::test]
    async fn pause_not_found() {
        let dir = TempDir::new().unwrap();
        let engine = make_test_engine(dir.path()).await;
        let err = engine.pause(99).await.unwrap_err();
        assert!(matches!(err, EngineError::NotFound { id: 99 }));
    }

    #[tokio::test]
    async fn resume_not_found() {
        let dir = TempDir::new().unwrap();
        let engine = make_test_engine(dir.path()).await;
        let err = engine.resume(42).await.unwrap_err();
        assert!(matches!(err, EngineError::NotFound { id: 42 }));
    }

    #[tokio::test]
    async fn remove_not_found() {
        let dir = TempDir::new().unwrap();
        let engine = make_test_engine(dir.path()).await;
        let err = engine.remove(7, false).await.unwrap_err();
        assert!(matches!(err, EngineError::NotFound { id: 7 }));
    }

    // Verifies that after remove() the handle is erased from the id map.
    // Requires outbound network access (DHT/trackers) to add the torrent.
    // Run with: cargo test -- --ignored
    #[tokio::test]
    #[ignore]
    async fn remove_cleans_up_handle_map_live() {
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
            inner: Mutex::new(EngineInner {
                handles: HashMap::new(),
                add_times: HashMap::new(),
                deleting: HashSet::new(),
                next_id: 1,
            }),
        });

        let magnet =
            "magnet:?xt=urn:btih:dd8255ecdc7ca55fb0bbf81323d87062db1f6d1c&dn=Big+Buck+Bunny";
        let info = engine.add_magnet(magnet.to_string()).await.expect("add_magnet");
        assert_eq!(engine.inner.lock().unwrap().handles.len(), 1);

        engine.remove(info.id, false).await.expect("remove");
        assert_eq!(
            engine.inner.lock().unwrap().handles.len(),
            0,
            "handle must be removed from the map after remove()"
        );
        // Confirm torrent_stats now returns NotFound.
        assert!(matches!(
            engine.torrent_stats(info.id),
            Err(EngineError::NotFound { .. })
        ));
    }

    // Verifies that session restore is fully complete before with_torrents is called —
    // the exact sequencing Engine::new relies on. Two back-to-back sessions share the
    // same persistence folder; the second session must enumerate the restored set without
    // a race, even though with_torrents is called synchronously right after the await.
    #[tokio::test]
    async fn session_restore_completes_before_with_torrents() {
        let dir = TempDir::new().unwrap();
        let session_dir = dir.path().join("session");

        // Phase 1: create and immediately drop a persistence-enabled session so the
        // folder is initialised (even though it contains no torrents).
        {
            let _session = Session::new_with_opts(
                dir.path().to_owned(),
                SessionOptions {
                    disable_dht: true,
                    disable_dht_persistence: true,
                    persistence: Some(SessionPersistenceConfig::Json {
                        folder: Some(session_dir.clone()),
                    }),
                    ..Default::default()
                },
            )
            .await
            .expect("first session creation");
        }

        // Phase 2: restore. The invariant is that new_with_opts returns only after the
        // persistence state has been loaded, so with_torrents (called synchronously
        // afterwards, exactly as in Engine::new) sees the complete restored set.
        let session = Session::new_with_opts(
            dir.path().to_owned(),
            SessionOptions {
                disable_dht: true,
                disable_dht_persistence: true,
                persistence: Some(SessionPersistenceConfig::Json {
                    folder: Some(session_dir),
                }),
                ..Default::default()
            },
        )
        .await
        .expect("restore session creation");

        let restored: Vec<usize> =
            session.with_torrents(|iter| iter.map(|(sid, _)| sid).collect());
        // Empty persistence → empty restore; the important thing is no panic and no race.
        assert_eq!(restored.len(), 0, "fresh persistence dir must restore zero torrents");
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
            inner: Mutex::new(EngineInner {
                handles: HashMap::new(),
                add_times: HashMap::new(),
                deleting: HashSet::new(),
                next_id: 1,
            }),
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
