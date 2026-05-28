import SwiftUI

struct AddMagnetSheet: View {
    @EnvironmentObject private var store: AppStore
    @Environment(\.dismiss) private var dismiss
    let onAdded: (TorrentVM) -> Void

    @State private var magnetText = ""
    @State private var errorMessage: String?
    @State private var isAdding = false
    @State private var addTask: Task<Void, Never>?

    var body: some View {
        VStack(spacing: 16) {
            Text("Add Torrent")
                .font(.headline)

            TextField("Paste magnet link…", text: $magnetText)
                .textFieldStyle(.roundedBorder)
                .frame(minWidth: 360)
                .disabled(isAdding)

            HStack {
                Button("Cancel") {
                    addTask?.cancel()
                    dismiss()
                }
                .keyboardShortcut(.cancelAction)

                Spacer()

                Button("Add") {
                    addTask = Task { await addMagnet() }
                }
                .keyboardShortcut(.defaultAction)
                .disabled(magnetText.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || isAdding)
            }
        }
        .padding()
        .alert("Error", isPresented: Binding(
            get: { errorMessage != nil },
            set: { if !$0 { errorMessage = nil } }
        )) {
            Button("OK", role: .cancel) { errorMessage = nil }
        } message: {
            Text(errorMessage ?? "")
        }
    }

    private func addMagnet() async {
        let uri = magnetText.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !uri.isEmpty else { return }

        isAdding = true
        defer { isAdding = false }

        do {
            if let vm = try await store.addMagnet(uri) {
                if Task.isCancelled {
                    try? await store.cancelTorrent(vm.id)
                    return
                }
                onAdded(vm)
                dismiss()
            } else {
                if !Task.isCancelled {
                    errorMessage = "Torrent is already in the list."
                }
            }
        } catch {
            if !Task.isCancelled {
                errorMessage = engineErrorMessage(error)
            }
        }
    }
}
