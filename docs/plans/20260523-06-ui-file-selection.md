# Plan: SwiftUI — File Selection Sheet

## Overview

After a magnet is added, present the user with the torrent's file tree and let them pick which files to download before anything starts streaming. Confirm calls `engine.setFileSelection`; cancel removes the torrent entirely so there are no zombie paused torrents in the list.

## Validation Commands

- `xcodebuild -project apps/TorrentApp.xcodeproj -scheme TorrentApp -configuration Debug build`
- Manual: add a multi-file magnet, deselect all but one file, confirm — only that file appears in `~/Downloads/TorrentApp/`. Add another magnet, click Cancel on the selection sheet — confirm no row is added and no files are written.

### Task 1: File selection sheet view

- [x] Create `Views/FileSelectionSheet.swift` taking a `TorrentVM` and `onConfirm` / `onCancel` closures.
- [x] Render rows with a `Toggle` per file, the file path, and the human-readable size; track selection state in a `@State Set<UInt32>` of indexes.
- [x] Add `Select all` / `Select none` controls at the top of the sheet.

### Task 2: Hook into the add flow

- [x] After `addMagnet` resolves successfully in `AppStore`, transition the UI to present `FileSelectionSheet` for the new torrent (instead of immediately appending to the visible list).
- [x] Only append the `TorrentVM` to `store.torrents` after the user confirms.
- [x] On cancel, call `engine.remove(id: id, deleteFiles: true)` and discard the VM.

### Task 3: Apply selection and guard against empty confirms

- [x] On confirm, call `await engine.setFileSelection(id:, selectedIndexes: Array(set))`.
- [x] Disable the confirm button when the selection set is empty.
- [x] Verify visually that file sizes are correct and totals match the torrent's `totalBytes`. [manual test - skipped, not automatable]
