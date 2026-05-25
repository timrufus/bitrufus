# Plan: Rust Engine — Stats, Pause, Resume, Remove

## Overview

Round out the engine API with everything Swift needs to drive the UI: a snapshot `TorrentStats` struct for progress polling, a `list_torrents` call for app launch, and `pause` / `resume` / `remove` operations. Last engine-side plan before SwiftUI work begins.

## Validation Commands

- `cargo build --release -p core`
- `cargo test -p core`
- `cargo clippy --all-targets -- -D warnings`
- Manual: poll `torrent_stats` once per second from a SwiftUI debug button — values increase during an active download, `download_speed_bps` falls to zero after `pause`, and the row vanishes after `remove`.

### Task 1: Stats snapshot type

- [x] Add `TorrentStats { id, state, downloaded_bytes, total_bytes, download_speed_bps, upload_speed_bps, peer_count }` to `core/src/types.rs`.
- [x] Define `TorrentState` enum with `Paused`, `Initializing`, `Downloading`, `Seeding`, `Error`; map exhaustively from librqbit's internal state.
- [x] Document the mapping in a comment so future librqbit upgrades that add states are caught by `cargo test`.

### Task 2: Read stats and list torrents

- [x] Implement `pub fn torrent_stats(&self, id: u64) -> Result<TorrentStats, EngineError>` reading from the handle's current stats.
- [x] Implement `pub fn list_torrents(&self) -> Vec<TorrentInfo>` returning info for every active id.
- [x] Both are synchronous `fn` — they read in-memory state with no I/O.

### Task 3: Pause, resume, remove

- [x] Implement `pub async fn pause(&self, id: u64) -> Result<(), EngineError>` and `pub async fn resume(&self, id: u64) -> Result<(), EngineError>`.
- [x] Implement `pub async fn remove(&self, id: u64, delete_files: bool) -> Result<(), EngineError>` calling librqbit's delete (with or without on-disk data), then removing the entry from the id map.
- [x] Confirm librqbit updates its persistence file so the torrent does not reappear after a relaunch.

### Task 4: Test and verify

- [ ] Write unit tests covering state-mapping exhaustiveness and id-map cleanup on remove.
- [ ] Add a test that calling `pause` / `resume` / `remove` on an unknown id returns `NotFound` rather than panicking.
- [ ] Run the full validation command set and the manual SwiftUI verification.
