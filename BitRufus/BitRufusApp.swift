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
