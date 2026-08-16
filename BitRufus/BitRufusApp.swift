//
//  BitRufusApp.swift
//  BitRufus
//
//  Created by Tim Rufus on 23.05.2026.
//

import SwiftUI
import AppKit

@main
struct BitRufusApp: App {
    @StateObject private var store = AppStore()
    @Environment(\.scenePhase) private var scenePhase

    init() {
        // Delegate must be installed before launch finishes so clicks on a
        // notification are handled even when they relaunch the app.
        DownloadNotifier.shared.activate()
    }

    var body: some Scene {
        WindowGroup {
            TorrentListView()
                .environmentObject(store)
                .onChange(of: scenePhase) { phase in
                    store.updateScenePhase(active: phase == .active)
                }
        }
        .commands {
            CommandGroup(after: .help) {
                Button("Show Logs in Finder") { revealLogFile() }
            }
        }
        Settings {
            SettingsView()
                .environmentObject(store)
        }
    }

    // The log lives inside the sandbox container, which a user cannot reach by hand.
    private func revealLogFile() {
        guard let path = logFilePath() else { return }
        let url = URL(fileURLWithPath: path)
        if FileManager.default.fileExists(atPath: path) {
            NSWorkspace.shared.activateFileViewerSelecting([url])
        } else {
            // Engine not started yet, so the file does not exist — open its folder instead.
            NSWorkspace.shared.open(url.deletingLastPathComponent())
        }
    }
}
