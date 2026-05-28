//
//  BitRufusApp.swift
//  BitRufus
//
//  Created by Тимофей Ермилов on 23.05.2026.
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
    }
}
