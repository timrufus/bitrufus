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

    var body: some Scene {
        WindowGroup {
            TorrentListView()
                .environmentObject(store)
        }
        Settings {
            SettingsView()
                .environmentObject(store)
        }
    }
}
