# Plan: Rust Engine — Session Lifecycle and Add Magnet

## Overview

Wrap librqbit's `Session` behind a small Rust-side `Engine` API exposed via UniFFI. After this plan, Swift can construct an `Engine` pointed at a download directory, add a magnet link, and receive a `TorrentInfo` describing the resolved torrent. Torrents are added paused — file selection happens in the next plan before any data is fetched.

## Validation Commands

- `cargo build --release -p core`
- `cargo test -p core`
- `cargo clippy --all-targets -- -D warnings`
- Manual: from a temporary SwiftUI debug button, call `Engine(downloadDir:).addMagnet(magnet:)` with a known public-domain magnet (Big Buck Bunny) and confirm a `TorrentInfo` with non-empty `name` and non-zero `totalBytes` is returned.

### Task 1: Define shared FFI types and error enum

- [ ] Create `core/src/types.rs` with a UniFFI-exported `TorrentInfo` record (`id`, `info_hash`, `name`, `total_bytes`).
- [ ] Define `EngineError` enum with `InvalidMagnet`, `NotFound`, `Io`, `Backend` variants, deriving `uniffi::Error` and `thiserror::Error`.
- [ ] Re-export types from `lib.rs`; confirm `cargo build` produces no UniFFI warnings or codegen issues.

### Task 2: Implement Engine with session and persistence

- [ ] Add `librqbit` and `directories` to `core/Cargo.toml` — verify the published version's API matches the calls below before coding.
- [ ] Create `core/src/engine.rs` with `Engine` as a `uniffi::Object` holding an `Arc<Session>`, an `AtomicU64` for ids, and a `Mutex<HashMap<u64, ManagedTorrentHandle>>`.
- [ ] Implement `#[uniffi::constructor] pub async fn new(download_dir: String)` creating the session with JSON persistence pointed at the macOS Application Support directory.
- [ ] Ensure the session restores any previously persisted torrents during construction and that the id map is populated from restored handles.

### Task 3: Implement add_magnet

- [ ] Add `pub async fn add_magnet(&self, magnet: String) -> Result<TorrentInfo, EngineError>` calling `session.add_torrent` with the torrent in paused state.
- [ ] Allocate a new id, insert the handle into the map, and return a populated `TorrentInfo`.
- [ ] Map librqbit failures into the appropriate `EngineError` variant; never panic on malformed input.

### Task 4: Test and verify

- [ ] Write unit tests covering: invalid magnet string returns `InvalidMagnet`; id allocation is monotonic.
- [ ] Mark live-network tests `#[ignore]` and add a comment showing the command to run them (`cargo test -- --ignored`).
- [ ] Run the full validation command set and the manual SwiftUI roundtrip.
