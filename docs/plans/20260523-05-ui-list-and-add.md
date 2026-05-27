# Plan: SwiftUI — Torrent List and Add-Magnet Sheet

## Overview

First SwiftUI plan: build the main window. A list of torrents fed by an `@Observable` store, a `+` toolbar button that opens a sheet for pasting a magnet link, and a placeholder row showing name and size. Live progress and file-selection sheets come in later plans — this plan just gets data flowing from Rust into the view.

## Validation Commands

- `xcodebuild -project apps/TorrentApp.xcodeproj -scheme TorrentApp -configuration Debug build`
- Manual: ⌘R, click `+`, paste a known magnet, click `Add` — confirm a row appears in the list with the torrent's name and total size. Add a second magnet; both rows visible.

### Task 1: Build the observable store

- [x] Create `apps/TorrentApp/ViewModels/AppStore.swift` with `@Observable class AppStore` owning a singleton `Engine` instance constructed pointing at `~/Downloads/TorrentApp/`.
- [x] Define `var torrents: [TorrentVM]` and `func addMagnet(_ uri: String) async throws`.
- [x] Create `TorrentVM` (also `@Observable`) holding `id`, the latest `TorrentInfo`, and an optional `TorrentStats` (populated by a later plan).

### Task 2: Torrent list view

- [x] Create `Views/TorrentListView.swift` rendering `List(store.torrents) { TorrentRow(vm: $0) }`.
- [x] Implement `TorrentRow` showing the name, total size formatted via `ByteCountFormatter`, and a placeholder `ProgressView(value: 0)`.
- [x] Add a toolbar with a `+` button that toggles a sheet binding.

### Task 3: Add-magnet sheet

- [ ] Create `Views/AddMagnetSheet.swift` with a `TextField`, an `Add` button, and a `Cancel` button.
- [ ] On `Add`, call `await store.addMagnet(text)` and dismiss; surface errors via a simple alert.
- [ ] Reject empty input client-side before calling the engine.

### Task 4: Wire into the app entry point

- [ ] In `TorrentAppApp.swift`, instantiate `AppStore` as `@State` at the top of the scene and inject into the environment.
- [ ] Replace `ContentView`'s body with `TorrentListView`.
- [ ] Run the validation commands and confirm the manual flow.
