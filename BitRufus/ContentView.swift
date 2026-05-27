//
//  ContentView.swift
//  BitRufus
//
//  Created by Тимофей Ермилов on 23.05.2026.
//

import SwiftUI

struct ContentView: View {
    var body: some View {
        TorrentListView()
    }
}

struct ContentView_Previews: PreviewProvider {
    static var previews: some View {
        ContentView()
            .environmentObject(AppStore())
    }
}
