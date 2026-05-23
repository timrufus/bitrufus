# Plan: Persistence Across Launches

## Overview

Make downloads survive app restarts. librqbit already persists the per-piece bitmap to its own JSON file; this plan ensures Swift correctly hydrates its UI state from that source plus a small side-file for UI-only fields (display name override, added-at timestamp).

## Validation Commands

- `xcodebuild -project apps/TorrentApp.xcodeproj -scheme TorrentApp -configuration Debug build`
- `cargo test -p core`
- Manual: start two downloads, wait until each is ~30% complete, quit the app, relaunch — both rows reappear immediately with the saved progress, and downloading resumes without re-fetching pieces already on disk.

### Task 1: Verify engine-side restore

- [ ] Confirm `Engine::new` blocks until librqbit's session restore finishes, so `list_torrents()` returns the full restored set without a race against the UI.
- [ ] Add a Rust integration test or doc comment describing the restore guarantee — future contributors should not have to re-derive it.

### Task 2: Side-file for UI metadata

- [ ] Create `Persistence/TorrentStore.swift` reading and writing `torrents.json` next to librqbit's session JSON.
- [ ] Store a map of `id → { displayName, addedAt }` and leave room to extend in later plans without schema breakage.
- [ ] Save on `addMagnet` success and on `remove`; load once during `AppStore.init`.

### Task 3: Hydrate the store on launch

- [ ] After `Engine` initialization, call `engine.listTorrents()` and build a `TorrentVM` for each, joining librqbit data with the side-file.
- [ ] Handle the edge case where the side-file has a stale id no longer known to the engine: drop the orphan entry silently and re-save the file.
- [ ] Run the manual verification — quitting mid-download and relaunching must preserve progress.
