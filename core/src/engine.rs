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
