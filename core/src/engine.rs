use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use directories::ProjectDirs;
use librqbit::{
    AddTorrent, AddTorrentOptions, AddTorrentResponse, ByteBufOwned, Magnet, ManagedTorrent,
    Session, SessionOptions, SessionPersistenceConfig, TorrentStatsState, torrent_from_bytes,
};

use crate::storage::no_prealloc_factory;
use crate::types::{EngineError, FileInfo, TorrentInfo, TorrentState, TorrentStats};

type TorrentHandle = Arc<ManagedTorrent>;

// Upper bound on how long a single add_magnet attempt waits for librqbit to resolve
// magnet metadata from peers/DHT/trackers before failing. This is per-attempt: the Swift
// layer auto-retries a failed resolve several times, because tracker DNS blocks are often
// intermittent and simply re-attempting catches a working window (how clients like Folx
// succeed). A shorter per-attempt bound therefore retries sooner instead of burning one
// long wait; long-but-live swarms still resolve within the window, and the UI shows
// elapsed time plus a "try the .torrent file" hint.
const MAGNET_RESOLVE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(120);

// Upper bound on how long set_file_selection waits for a freshly-added torrent to finish
// librqbit's Initializing phase (file integrity check / allocation) before applying the
// file selection. update_only_files bails with "can't update initializing torrent" if
// called during that window.
const INIT_WAIT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

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
        init_logging();

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
            // Open a TCP listener for inbound peer connections. Without listen_port_range
            // librqbit never binds a socket, announces port=0 to trackers/DHT, and can only
            // connect outbound — which yields few or zero peers on many swarms (peers behind
            // NAT, small swarms), leaving downloads stuck at 0 bytes. The sandbox already
            // grants com.apple.security.network.server for exactly this. UPnP forwarding
            // helps home-router peers reach us.
            listen_port_range: Some(6881..6890),
            enable_upnp_port_forwarding: true,
            // Skip upfront full-file preallocation so downloads to exFAT/FAT external drives
            // (no sparse-file support) don't freeze while zero-filling the whole file. See
            // crate::storage for the rationale.
            default_storage_factory: Some(no_prealloc_factory()),
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

        // librqbit's add_torrent blocks until magnet metadata is resolved from
        // peers/DHT/trackers (paused does not skip this). A magnet with no reachable
        // peers (e.g. a private-tracker-only link) would otherwise hang forever, so
        // bound the resolve with a timeout and surface a clear error instead.
        let add_future = self.session.add_torrent(
            AddTorrent::from_url(&magnet),
            Some(AddTorrentOptions {
                paused: true,
                // Reuse/resume existing files on disk instead of failing when a file already
                // exists (librqbit's create_new default). Matches how librqbit restores
                // persisted torrents (into_add_torrent sets overwrite: true) and how other
                // clients resume; pieces are hash-checked, so existing data is not clobbered.
                overwrite: true,
                ..Default::default()
            }),
        );
        let response = tokio::time::timeout(MAGNET_RESOLVE_TIMEOUT, add_future)
            .await
            .map_err(|_| EngineError::Backend {
                reason: "Couldn't find any peers for this magnet. The tracker may be unreachable on your network, or there are no seeders — adding the .torrent file instead is more reliable.".to_string(),
            })?
            .map_err(|e| EngineError::Backend {
                reason: e.to_string(),
            })?;

        // For paused magnet-only adds, metadata (name, size) may not be resolved yet;
        // fall back to the dn= display name captured above.
        self.register_added_handle(response, dn)
    }

    pub async fn add_torrent_file(&self, bytes: Vec<u8>) -> Result<TorrentInfo, EngineError> {
        // Pre-validate so parse failures map to InvalidTorrent and session failures map to Backend.
        torrent_from_bytes::<ByteBufOwned>(&bytes).map_err(|e| EngineError::InvalidTorrent {
            reason: e.to_string(),
        })?;
        let response = self
            .session
            .add_torrent(
                AddTorrent::from_bytes(bytes),
                Some(AddTorrentOptions {
                    paused: true,
                    // Reuse/resume existing files rather than failing when they already exist
                    // (e.g. re-adding a torrent whose data is already partly downloaded).
                    // Same setting librqbit uses when restoring persisted torrents.
                    overwrite: true,
                    ..Default::default()
                }),
            )
            .await
            .map_err(|e| EngineError::Backend {
                reason: e.to_string(),
            })?;
        // .torrent files embed the full info dict, so no fallback name is needed.
        self.register_added_handle(response, None)
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

        // A freshly-added torrent (even one added paused) transitions through
        // Initializing (file integrity check / allocation) before settling into Paused.
        // librqbit's update_only_files bails with "can't update initializing torrent"
        // during that window, so wait for the torrent to leave Initializing — which for a
        // paused add is monotonic (Initializing -> Paused, never back) — before applying
        // the selection. Without this, clicking Download right after add races the init
        // and silently leaves the torrent unselected and paused.
        self.wait_until_left_initializing(&handle).await?;

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

    pub fn all_stats(&self) -> Vec<TorrentStats> {
        // Snapshot handles and release the lock before calling handle.stats() so
        // concurrent add/remove callers are not blocked for the full iteration.
        let snapshot: Vec<(u64, TorrentHandle)> = {
            let inner = self.inner.lock().expect("inner lock poisoned");
            inner.handles.iter().map(|(&id, h)| (id, h.clone())).collect()
        };
        snapshot.into_iter().map(|(id, handle)| stats_from_handle(id, &handle)).collect()
    }

    pub async fn pause(&self, id: u64) -> Result<(), EngineError> {
        let handle = {
            let inner = self.inner.lock().expect("inner lock poisoned");
            inner.handles.get(&id).cloned().ok_or(EngineError::NotFound { id })?
        };
        match self.session.pause(&handle).await {
            Ok(()) => Ok(()),
            // librqbit reports "already paused" as a plain (string-only) anyhow error.
            // Rather than match on that unstable message, confirm the desired end-state:
            // if the torrent is now paused the call was effectively a no-op success. Real
            // failures (e.g. "can't pause initializing torrent") leave it unpaused and
            // still propagate.
            Err(_) if handle.is_paused() => Ok(()),
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

    pub fn torrent_info(&self, id: u64) -> Result<TorrentInfo, EngineError> {
        let handle = {
            let inner = self.inner.lock().expect("inner lock poisoned");
            inner.handles.get(&id).cloned().ok_or(EngineError::NotFound { id })?
        };
        Ok(TorrentInfo {
            id,
            name: handle.name().unwrap_or_default(),
            total_bytes: handle.stats().total_bytes,
        })
    }

    pub fn torrent_files(&self, id: u64) -> Result<Vec<FileInfo>, EngineError> {
        let handle = {
            let inner = self.inner.lock().expect("inner lock poisoned");
            inner.handles.get(&id).cloned().ok_or_else(|| {
                let known: Vec<u64> = inner.handles.keys().copied().collect();
                tracing::warn!(id, ?known, "torrent_files: id not found in handles");
                EngineError::NotFound { id }
            })?
        };

        let only_files = handle.only_files();

        let result = handle
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
            });
        match &result {
            Ok(files) => tracing::debug!(id, count = files.len(), "torrent_files ok"),
            Err(e) => tracing::warn!(id, error = ?e, "torrent_files failed"),
        }
        result
    }
}

// Installs a tracing subscriber once so librqbit's diagnostics (tracker announces, peer
// connections, disk-write errors, etc.) surface on stderr — visible in Console.app and the
// Xcode console. Default verbosity keeps librqbit at info; override with RUST_LOG, e.g.
// `RUST_LOG=librqbit=debug`. Safe to call repeatedly: try_init is a no-op after the first.
fn init_logging() {
    use std::sync::Once;
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        use tracing_subscriber::EnvFilter;
        let filter = EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| EnvFilter::new("librqbit=info,bitrufus_core=debug"));
        let _ = tracing_subscriber::fmt()
            .with_env_filter(filter)
            .with_writer(std::io::stderr)
            .try_init();
    });
}

fn stats_from_handle(id: u64, handle: &TorrentHandle) -> TorrentStats {
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
    TorrentStats {
        id,
        state,
        downloaded_bytes: rq.progress_bytes,
        total_bytes: rq.total_bytes,
        download_speed_bps,
        upload_speed_bps,
        peer_count,
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
    // Resolves the handle from an AddTorrentResponse, applies the deleting-tombstone check
    // and ptr_eq dedup, allocates an engine ID (or returns the existing one), and builds
    // TorrentInfo. Shared by add_magnet and add_torrent_file so the concurrency-critical
    // block lives in exactly one place.
    fn register_added_handle(
        &self,
        response: AddTorrentResponse,
        fallback_name: Option<String>,
    ) -> Result<TorrentInfo, EngineError> {
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
        let name = handle.name().or(fallback_name).unwrap_or_default();

        Ok(TorrentInfo {
            id,
            name,
            total_bytes: stats.total_bytes,
        })
    }

    // Waits for a freshly-added torrent to leave the Initializing state (librqbit's
    // file-integrity-check / allocation phase) before file selection is applied. For a
    // paused add the transition is monotonic (Initializing -> Paused, never back), so once
    // the state is anything other than Initializing it is safe to call update_only_files.
    // Bounded so a torrent stuck initializing surfaces an error instead of hanging the UI.
    async fn wait_until_left_initializing(
        &self,
        handle: &TorrentHandle,
    ) -> Result<(), EngineError> {
        let start = std::time::Instant::now();
        loop {
            if !matches!(handle.stats().state, TorrentStatsState::Initializing) {
                return Ok(());
            }
            if start.elapsed() >= INIT_WAIT_TIMEOUT {
                return Err(EngineError::Backend {
                    reason: "torrent is still initializing after 30s".to_string(),
                });
            }
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        }
    }

    // Calls session.unpause idempotently. librqbit returns a plain (string-only) anyhow
    // error ("torrent is already live") when unpause is called on a running torrent; rather
    // than match that unstable message, we confirm via state — if the torrent ended up
    // unpaused the call effectively succeeded. Real failures leave it paused and propagate.
    async fn unpause_idempotent(&self, handle: &TorrentHandle) -> Result<(), EngineError> {
        match self.session.unpause(handle).await {
            Ok(()) => Ok(()),
            Err(_) if !handle.is_paused() => Ok(()),
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
    async fn list_torrents_empty() {
        let dir = TempDir::new().unwrap();
        let engine = make_test_engine(dir.path()).await;
        assert_eq!(engine.list_torrents().len(), 0);
    }

    #[tokio::test]
    async fn all_stats_empty_when_no_handles() {
        let dir = TempDir::new().unwrap();
        let engine = make_test_engine(dir.path()).await;
        assert_eq!(engine.all_stats().len(), 0);
    }

    #[tokio::test]
    async fn torrent_info_not_found() {
        let dir = TempDir::new().unwrap();
        let engine = make_test_engine(dir.path()).await;
        let err = engine.torrent_info(42).unwrap_err();
        assert!(matches!(err, EngineError::NotFound { id: 42 }));
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

    // Builds a minimal valid single-file .torrent as raw bencoded bytes.
    // Structure: d4:infod6:lengthi1024e4:name8:test.txt12:piece lengthi16384e6:pieces20:<20 bytes>ee
    // Keys are in lexicographic order as required by the bencode spec.
    // One piece (ceil(1024/16384) = 1) → pieces field is exactly 20 bytes.
    fn minimal_torrent_bytes() -> Vec<u8> {
        let mut v = Vec::new();
        v.extend_from_slice(
            b"d4:infod6:lengthi1024e4:name8:test.txt12:piece lengthi16384e6:pieces20:",
        );
        v.extend_from_slice(&[0u8; 20]);
        v.extend_from_slice(b"ee");
        v
    }

    #[tokio::test]
    async fn corrupt_torrent_bytes_returns_invalid_torrent() {
        let dir = TempDir::new().unwrap();
        let engine = make_test_engine(dir.path()).await;
        let err = engine.add_torrent_file(b"not a torrent".to_vec()).await.unwrap_err();
        assert!(matches!(err, EngineError::InvalidTorrent { .. }));
    }

    #[tokio::test]
    async fn valid_torrent_file_returns_torrent_info() {
        let dir = TempDir::new().unwrap();
        let engine = make_test_engine(dir.path()).await;
        let info = engine.add_torrent_file(minimal_torrent_bytes()).await.unwrap();
        assert_eq!(info.name, "test.txt", "name must match the info dict name field");
        assert!(info.total_bytes > 0, "total_bytes must be populated immediately from .torrent");
    }

    // Regression for the "storages other than FilesystemStorageFactory are not supported"
    // bail: adding a .torrent file with JSON session persistence enabled (as the real app
    // runs) plus the NoPrealloc storage factory must succeed. Metadata is embedded in the
    // .torrent, so persistence runs on add and hits librqbit's storage-type gate; the
    // factory claims the FilesystemStorageFactory type id so it passes. make_test_engine
    // has persistence disabled, which is why the other add_torrent_file tests miss this.

    #[tokio::test]
    async fn torrent_file_persists_with_no_prealloc_factory() {
        let dir = TempDir::new().unwrap();
        let session = Session::new_with_opts(
            dir.path().to_owned(),
            SessionOptions {
                disable_dht: true,
                disable_dht_persistence: true,
                persistence: Some(SessionPersistenceConfig::Json {
                    folder: Some(dir.path().join("session")),
                }),
                default_storage_factory: Some(no_prealloc_factory()),
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

        let info = engine
            .add_torrent_file(minimal_torrent_bytes())
            .await
            .expect("add must succeed with persistence + no-prealloc factory");
        assert_eq!(info.name, "test.txt");
    }

    #[tokio::test]
    async fn duplicate_torrent_file_returns_same_id() {
        let dir = TempDir::new().unwrap();
        let engine = make_test_engine(dir.path()).await;
        let info1 = engine.add_torrent_file(minimal_torrent_bytes()).await.unwrap();
        let info2 = engine.add_torrent_file(minimal_torrent_bytes()).await.unwrap();
        assert_eq!(info1.id, info2.id, "duplicate add must return the same engine id");
        assert_eq!(
            engine.inner.lock().unwrap().handles.len(),
            1,
            "duplicate add must not create a second handle"
        );
    }

    // Verifies that the deleting-tombstone check in register_added_handle applies to
    // add_torrent_file (shared code path; also covers add_magnet via the same helper).
    #[tokio::test]
    async fn add_torrent_file_blocked_during_delete() {
        let dir = TempDir::new().unwrap();
        let engine = make_test_engine(dir.path()).await;
        let info = engine.add_torrent_file(minimal_torrent_bytes()).await.unwrap();

        // Simulate an in-flight deletion by inserting the librqbit session ID into deleting.
        let librqbit_id = {
            let inner = engine.inner.lock().unwrap();
            inner.handles.get(&info.id).unwrap().id()
        };
        engine.inner.lock().unwrap().deleting.insert(librqbit_id);

        // A second add of the same bytes returns AlreadyManaged, which then hits the
        // deleting check and must return a Backend error rather than a zombie handle.
        let err = engine.add_torrent_file(minimal_torrent_bytes()).await.unwrap_err();
        assert!(
            matches!(err, EngineError::Backend { .. }),
            "expected Backend error for add-during-delete, got {err:?}"
        );

        // Clean up so the engine is left in a consistent state.
        engine.inner.lock().unwrap().deleting.remove(&librqbit_id);
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
        // Confirm torrent_info now returns NotFound.
        assert!(matches!(
            engine.torrent_info(info.id),
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

    // Regression test for NoPreallocStorageFactory: it must skip librqbit's upfront
    // set_len(full_size). After a paused add, the default backend reserves the torrent's full
    // declared length (1024 bytes here), while the no-prealloc backend leaves the file at 0 —
    // it grows only as pieces are written. On exFAT/FAT that reserved length is physically
    // zero-filled, which is the freeze this backend avoids.
    async fn file_size_after_paused_add(use_factory: bool) -> u64 {
        let dir = TempDir::new().unwrap();
        let session = Session::new_with_opts(
            dir.path().to_owned(),
            SessionOptions {
                disable_dht: true,
                disable_dht_persistence: true,
                default_storage_factory: use_factory.then(no_prealloc_factory),
                ..Default::default()
            },
        )
        .await
        .expect("session");
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
        let info = engine.add_torrent_file(minimal_torrent_bytes()).await.expect("add");
        let handle = engine.inner.lock().unwrap().handles.get(&info.id).unwrap().clone();
        engine.wait_until_left_initializing(&handle).await.ok();
        // minimal_torrent_bytes names the single file "test.txt" with length 1024.
        std::fs::metadata(dir.path().join("test.txt")).map(|m| m.len()).unwrap_or(0)
    }

    #[tokio::test]
    async fn default_backend_preallocates_full_length() {
        assert_eq!(file_size_after_paused_add(false).await, 1024);
    }

    #[tokio::test]
    async fn no_prealloc_backend_skips_preallocation() {
        assert_eq!(file_size_after_paused_add(true).await, 0);
    }
}
