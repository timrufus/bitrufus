import SwiftUI

struct AddMagnetSheet: View {
    @EnvironmentObject private var store: AppStore
    @Environment(\.dismiss) private var dismiss

    @State private var magnetText = ""

    private var trimmed: String {
        magnetText.trimmingCharacters(in: .whitespacesAndNewlines)
    }

    var body: some View {
        VStack(spacing: 16) {
            Text("Add Torrent")
                .font(.headline)

            TextField("Paste magnet link…", text: $magnetText)
                .textFieldStyle(.roundedBorder)
                .frame(minWidth: 360)
                .onSubmit(add)

            HStack {
                Button("Cancel") {
                    dismiss()
                }
                .keyboardShortcut(.cancelAction)

                Spacer()

                Button("Add", action: add)
                    .keyboardShortcut(.defaultAction)
                    .disabled(trimmed.isEmpty)
            }
        }
        .padding()
    }

    // The magnet appears in the list as a resolving row right away — no blocking here,
    // so the sheet just hands off and closes.
    private func add() {
        guard !trimmed.isEmpty else { return }
        store.beginAddMagnet(trimmed)
        dismiss()
    }
}
