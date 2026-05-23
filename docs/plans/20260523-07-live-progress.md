# Plan: Live Progress Polling

## Overview

Wire stats from the engine into the UI on a 500 ms timer. Each row shows a real progress bar, current download speed, and peer count, all updating live. This is the polling pump the rest of the read-only UI hangs off of.

## Validation Commands

- `xcodebuild -project apps/TorrentApp.xcodeproj -scheme TorrentApp -configuration Debug build`
- Manual: start a download, watch the progress bar fill in real time; speed and peer count update at least once per second; with three simultaneous downloads, each row updates independently.

### Task 1: Polling task in the store

- [ ] In `AppStore.init`, spawn a `Task` that runs `for await _ in Timer.publish(every: 0.5, on: .main, in: .common).autoconnect().values { refreshStats() }`.
- [ ] `refreshStats()` iterates the current torrents and calls `engine.torrentStats(id:)` for each, assigning the result to `vm.stats`.
- [ ] Cancel the task in a `deinit`-equivalent or on app exit so it does not leak when the store is reconstructed.

### Task 2: Render progress in the row

- [ ] Replace the placeholder `ProgressView` in `TorrentRow` with one driven by `Double(stats.downloadedBytes) / Double(stats.totalBytes)`.
- [ ] Add a subtitle line showing formatted speed (`ByteCountFormatter` + "/s") and peer count, e.g. "2.3 MB/s · 12 peers".
- [ ] Show a state badge when the torrent is not actively downloading (Paused, Seeding, Error).

### Task 3: Verify under load

- [ ] Add three magnets simultaneously and confirm all three rows update independently and accurately.
- [ ] Eyeball CPU and memory in Activity Monitor with three active downloads and confirm the polling loop is not a hotspot.
- [ ] If stats reads block the main thread visibly, move the loop body to a detached task and dispatch only the assignment back to main.
