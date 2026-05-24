//
//  ContentView.swift
//  BitRufus
//
//  Created by Тимофей Ермилов on 23.05.2026.
//

import SwiftUI

private struct TorrentEntry: Identifiable {
    let id = UUID()
    let name: String
}

struct ContentView: View {
    @State private var magnetText = ""
    @State private var torrents: [TorrentEntry] = []
    @State private var rustPing = ""

    var body: some View {
        VStack(spacing: 12) {
            HStack {
                TextField("Paste magnet link…", text: $magnetText)
                    .textFieldStyle(.roundedBorder)
                Button("Add") {
                    let name = magnetText.isEmpty ? "untitled" : String(magnetText.prefix(40))
                    torrents.append(TorrentEntry(name: name))
                    magnetText = ""
                }
            }
            List(torrents) { entry in
                HStack {
                    Text(entry.name)
                    Spacer()
                    ProgressView(value: 0.3)
                        .frame(width: 120)
                }
            }
        }
        .padding()
        .frame(minWidth: 500, minHeight: 300)
        .onAppear { rustPing = ping() }
        .safeAreaInset(edge: .bottom) {
            Text("Rust: \(rustPing)")
                .font(.caption2)
                .foregroundStyle(.secondary)
                .padding(4)
                .accessibilityIdentifier("rust-ping-label")
        }
    }
}

struct ContentView_Previews: PreviewProvider {
    static var previews: some View {
        ContentView()
    }
}
