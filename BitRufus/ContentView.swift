//
//  ContentView.swift
//  BitRufus
//
//  Created by Тимофей Ермилов on 23.05.2026.
//

import SwiftUI

struct ContentView: View {
    @State private var magnetText = ""
    @State private var torrents: [String] = []

    var body: some View {
        VStack(spacing: 12) {
            HStack {
                TextField("Paste magnet link…", text: $magnetText)
                    .textFieldStyle(.roundedBorder)
                Button("Add") {
                    torrents.append(magnetText.isEmpty ? "untitled" : String(magnetText.prefix(40)))
                    magnetText = ""
                }
            }
            List(torrents, id: \.self) { name in
                HStack {
                    Text(name)
                    Spacer()
                    ProgressView(value: 0.3)
                        .frame(width: 120)
                }
            }
        }
        .padding()
        .frame(minWidth: 500, minHeight: 300)
    }
}

struct ContentView_Previews: PreviewProvider {
    static var previews: some View {
        ContentView()
    }
}
