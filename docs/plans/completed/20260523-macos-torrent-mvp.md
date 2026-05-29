# macOS Torrent Client MVP — Index

## Overview

End-to-end MVP for a native macOS torrent client (similar to Folx). SwiftUI front-end + Rust core wrapping [librqbit](https://github.com/ikatson/rqbit), bridged via [UniFFI](https://mozilla.github.io/uniffi-rs/). Scope: paste a magnet, pick which files to download, watch live progress, pause/resume/remove, survive a relaunch with downloads resuming. HTTP/FTP downloads, settings UI, bandwidth limits, RSS, and notarization are explicitly out of scope.

Execute plans in numerical order. Plans 02–04 (Rust engine) must finish before plans 05+ (SwiftUI) — the Swift side imports UniFFI bindings generated from the engine surface. After execution each plan moves to `docs/plans/completed/`.

## Plans

1. [20260523-01-foundation-skeleton.md](20260523-01-foundation-skeleton.md) — Cargo workspace, Xcode project, UniFFI hello-world roundtrip.
2. [20260523-02-engine-session.md](20260523-02-engine-session.md) — `Engine` with session lifecycle and `add_magnet`.
3. [20260523-03-engine-file-selection.md](20260523-03-engine-file-selection.md) — `torrent_files` listing and `set_file_selection`.
4. [20260523-04-engine-stats-controls.md](20260523-04-engine-stats-controls.md) — Stats snapshot, pause, resume, remove.
5. [20260523-05-ui-list-and-add.md](20260523-05-ui-list-and-add.md) — Observable store, torrent list, add-magnet sheet.
6. [20260523-06-ui-file-selection.md](20260523-06-ui-file-selection.md) — File picker sheet wired into the add flow.
7. [20260523-07-live-progress.md](20260523-07-live-progress.md) — 500 ms polling pump driving progress, speed, peer count.
8. [20260523-08-ui-controls.md](20260523-08-ui-controls.md) — Pause/resume context menu + remove confirmation dialog.
9. [20260523-09-persistence.md](20260523-09-persistence.md) — Surviving relaunches via librqbit session restore + UI side-file.
10. [20260523-10-readme-and-handoff.md](20260523-10-readme-and-handoff.md) — README, screenshot, post-MVP roadmap.

## Stack Decisions

- **UI**: SwiftUI on macOS 14+ (Sonoma) using `@Observable`.
- **Engine**: Rust `core` crate built as a static library; deps include `librqbit` and `tokio` (multi-thread runtime).
- **Bridge**: UniFFI proc-macro mode — no `.udl` file, async Rust functions surface as Swift `async throws`.
- **Progress**: pull-based 500 ms polling, not push callbacks (simpler FFI, plenty for human eyes).
- **Persistence**: librqbit's `SessionPersistenceConfig::Json` in `~/Library/Application Support/TorrentApp/`, plus a Swift-owned `torrents.json` side-file for UI-only fields.
- **Downloads land in**: `~/Downloads/TorrentApp/` (hardcoded; settings UI is post-MVP).

## Out of Scope (Post-MVP)

HTTP/FTP downloads, settings UI, bandwidth limits, sequential download, notifications, dock badge, code signing & notarization, RSS, scheduling, multi-language UI.
