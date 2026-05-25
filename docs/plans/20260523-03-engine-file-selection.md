# Plan: Rust Engine — File Listing and Selection

## Overview

Expose per-file metadata and selection control on the engine. Swift can query a paused torrent's file tree, pass back the subset of files the user wants, and the engine narrows librqbit's download set to exactly those files. Downloads only begin after `set_file_selection` runs — opening the door for the file-picker UI to choose first, download second.

## Validation Commands

- `cargo build --release -p core`
- `cargo test -p core`
- `cargo clippy --all-targets -- -D warnings`
- Manual: from SwiftUI, add a multi-file magnet, log `torrent_files(id)`, call `set_file_selection(id, [0])`, observe only the first file appearing under `~/Downloads/TorrentApp/`.

### Task 1: Define FileInfo and listing API

- [x] Add `FileInfo { index, path, size_bytes, selected }` to `core/src/types.rs`.
- [x] Implement `pub fn torrent_files(&self, id: u64) -> Result<Vec<FileInfo>, EngineError>` reading the resolved torrent info and the current selection state.
- [x] Return `NotFound` if the id is not in the map; never panic on a stale id from the UI.

### Task 2: Implement file selection

- [ ] Add `pub async fn set_file_selection(&self, id: u64, selected_indexes: Vec<u32>) -> Result<(), EngineError>` that calls librqbit's update-only-files API (verify the exact method name against the installed version before writing the call).
- [ ] Unpause the torrent after selection is applied if at least one file is selected.
- [ ] Treat an empty selection as a no-op (do not unpause) — never start a download with zero files.

### Task 3: Test and verify

- [ ] Write unit tests for any local conversion logic (e.g. dedup, sort, out-of-range index rejection) and for the empty-selection no-op behavior.
- [ ] Add an `#[ignore]`-marked integration test that adds a real magnet, lists files, selects one, and asserts the others are not written to disk.
- [ ] Run the full validation command set and the manual SwiftUI verification.
