//
//  BitRufusApp.swift
//  BitRufus
//
//  Created by Tim Rufus on 23.05.2026.
//

import SwiftUI

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
        Settings {
            SettingsView()
                .environmentObject(store)
        }
    }
}
