import SwiftUI
import AppKit

struct SettingsView: View {
    @EnvironmentObject private var store: AppStore
    @State private var displayPath = AppSettings.shared.displayPath

    var body: some View {
        Form {
            Section("Downloads") {
                HStack {
                    Text(displayPath)
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                        .truncationMode(.middle)
                    Spacer()
                    Button("Choose…") { pickFolder() }
                }
            }
        }
        .formStyle(.grouped)
        .frame(width: 420)
        .padding()
    }

    private func pickFolder() {
        let panel = NSOpenPanel()
        panel.canChooseFiles = false
        panel.canChooseDirectories = true
        panel.canCreateDirectories = true
        panel.prompt = "Select"
        guard panel.runModal() == .OK, let url = panel.url else { return }
        AppSettings.shared.saveDirectory(url)
        displayPath = url.path
        store.restartEngine()
    }
}
