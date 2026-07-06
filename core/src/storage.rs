//! Storage backend that disables librqbit's upfront full-file preallocation.
//!
//! librqbit's default `FilesystemStorage` calls `set_len(full_file_size)` during
//! initialization (`ensure_file_length`). On APFS that produces an instant sparse file
//! (zero bytes actually on disk), but on filesystems without sparse support — exFAT/FAT,
//! common on external drives — it physically writes the entire file of zeros *before* the
//! download can start. For a 13 GB torrent on exFAT that is ~20 s of solid disk writing
//! during which the torrent is stuck "Initializing" and the app appears frozen.
//!
//! librqbit picks pieces in natural (sequential) order, so preallocation is unnecessary:
//! the file grows incrementally as pieces are written. This wrapper delegates everything to
//! the real `FilesystemStorage` except `ensure_file_length`, which becomes a no-op — the
//! same incremental-growth behavior used by Folx/Transmission/qBittorrent by default.

use std::any::TypeId;
use std::path::Path;

use librqbit::storage::filesystem::FilesystemStorageFactory;
use librqbit::storage::{BoxStorageFactory, StorageFactory, TorrentStorage};
use librqbit::{ManagedTorrentShared, TorrentMetadata};

/// Builds the `BoxStorageFactory` to install as the session's `default_storage_factory`.
///
/// We box the factory directly (`Box::new`) rather than via `StorageFactoryExt::boxed()`:
/// the `.boxed()` wrapper computes `is_type_id` from the wrapped factory's intrinsic
/// `TypeId`, which would shadow our override below and make session persistence reject
/// every torrent using this factory.
pub fn no_prealloc_factory() -> BoxStorageFactory {
    Box::new(NoPreallocStorageFactory)
}

#[derive(Default, Clone, Copy)]
pub struct NoPreallocStorageFactory;

impl StorageFactory for NoPreallocStorageFactory {
    // Produce boxed storage directly so this factory *is* a
    // `StorageFactory<Storage = Box<dyn TorrentStorage>>` and can be used as a
    // `BoxStorageFactory` without the `.boxed()` wrapper (see `no_prealloc_factory`).
    type Storage = Box<dyn TorrentStorage>;

    fn create(
        &self,
        shared: &ManagedTorrentShared,
        metadata: &TorrentMetadata,
    ) -> anyhow::Result<Box<dyn TorrentStorage>> {
        let inner = FilesystemStorageFactory::default().create(shared, metadata)?;
        Ok(Box::new(NoPreallocStorage {
            inner: Box::new(inner),
        }))
    }

    // This factory delegates entirely to `FilesystemStorage`, so it is filesystem-backed
    // for every purpose librqbit cares about. librqbit's JSON session persistence gates on
    // `storage_factory.is_type_id(FilesystemStorageFactory)` (session_persistence/json.rs)
    // and bails otherwise — so without claiming that type id, adding a `.torrent` file
    // (metadata resolves immediately → persistence runs on add) fails with
    // "storages other than FilesystemStorageFactory are not supported". Restore does not
    // use the stored storage type; it recreates torrents via the session's
    // `default_storage_factory` (this factory), so claiming the type id is safe.
    fn is_type_id(&self, type_id: TypeId) -> bool {
        type_id == TypeId::of::<FilesystemStorageFactory>() || type_id == TypeId::of::<Self>()
    }

    fn clone_box(&self) -> BoxStorageFactory {
        no_prealloc_factory()
    }
}

pub struct NoPreallocStorage {
    inner: Box<dyn TorrentStorage>,
}

impl TorrentStorage for NoPreallocStorage {
    fn pread_exact(&self, file_id: usize, offset: u64, buf: &mut [u8]) -> anyhow::Result<()> {
        self.inner.pread_exact(file_id, offset, buf)
    }

    fn pwrite_all(&self, file_id: usize, offset: u64, buf: &[u8]) -> anyhow::Result<()> {
        self.inner.pwrite_all(file_id, offset, buf)
    }

    fn remove_file(&self, file_id: usize, filename: &Path) -> anyhow::Result<()> {
        self.inner.remove_file(file_id, filename)
    }

    fn remove_directory_if_empty(&self, path: &Path) -> anyhow::Result<()> {
        self.inner.remove_directory_if_empty(path)
    }

    // The whole point of this wrapper: skip preallocation. Pieces are written in sequential
    // order, so the file grows incrementally — no upfront zero-fill on non-sparse filesystems.
    fn ensure_file_length(&self, _file_id: usize, _length: u64) -> anyhow::Result<()> {
        Ok(())
    }

    fn init(
        &mut self,
        shared: &ManagedTorrentShared,
        metadata: &TorrentMetadata,
    ) -> anyhow::Result<()> {
        self.inner.init(shared, metadata)
    }

    fn take(&self) -> anyhow::Result<Box<dyn TorrentStorage>> {
        Ok(Box::new(NoPreallocStorage {
            inner: self.inner.take()?,
        }))
    }
}
