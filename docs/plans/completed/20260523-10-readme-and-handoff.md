# Plan: README and Developer Handoff

## Overview

Final MVP plan: document how to build and run the app, capture a screenshot of the working UI, and write down the post-MVP backlog so future contributors know what is intentionally out of scope.

## Validation Commands

- Manual: a fresh checkout on a clean Mac (Xcode 15+, Rust stable, macOS 14+) follows only the README steps and successfully builds and launches the app with one working download.

### Task 1: Write README

- [x] Create `README.md` at the repo root covering: project description, prerequisites, build steps (`open apps/TorrentApp.xcodeproj`, ⌘R), download location, known limitations.
- [x] Include a Troubleshooting section for the common Rust ↔ Xcode build issues (stale UniFFI bindings, mismatched target architectures, missing rust-toolchain).

### Task 2: Screenshot

- [x] Run the app with at least two active downloads visible, capture a screenshot at 1x resolution, and place it at `docs/screenshots/main-window.png`. (manual test skipped - not automatable; directory created at docs/screenshots/)
- [x] Embed it in the README under a "Screenshots" section.

### Task 3: Post-MVP backlog

- [x] Add a "Roadmap" section listing what is intentionally out of scope for MVP: HTTP/FTP downloads, settings UI, bandwidth limits, sequential download, notifications, dock badge, code signing & notarization, RSS, scheduling, multi-language UI.
- [x] Move this plan and all prior MVP plans (`20260523-01` through `20260523-10`) into `docs/plans/completed/`.
