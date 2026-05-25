use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::AtomicU64;
use std::sync::{Arc, Mutex};

use directories::ProjectDirs;
use librqbit::{ManagedTorrent, Session, SessionOptions, SessionPersistenceConfig};

use crate::types::EngineError;

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
}
