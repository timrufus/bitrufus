import SwiftUI

struct FileSelectionSheet: View {
    let vm: TorrentVM
    let files: [FileInfo]
    let onConfirm: ([UInt32]) -> Void
    let onCancel: () -> Void

    @State private var selectedIndexes: Set<UInt32>

    private static let byteFormatter: ByteCountFormatter = {
        let f = ByteCountFormatter()
        f.countStyle = .file
        return f
    }()

    init(vm: TorrentVM, files: [FileInfo], onConfirm: @escaping ([UInt32]) -> Void, onCancel: @escaping () -> Void) {
        self.vm = vm
        self.files = files
        self.onConfirm = onConfirm
        self.onCancel = onCancel
        _selectedIndexes = State(initialValue: Set(files.filter(\.selected).map(\.index)))
    }

    var body: some View {
        VStack(spacing: 0) {
            Text(vm.info.name.isEmpty ? "(Unknown)" : vm.info.name)
                .font(.headline)
                .padding()

            Divider()

            HStack {
                Button("Select all") {
                    selectedIndexes = Set(files.map(\.index))
                }
                Button("Select none") {
                    selectedIndexes = []
                }
                Spacer()
            }
            .padding(.horizontal)
            .padding(.vertical, 8)

            Divider()

            List(files, id: \.index) { file in
                Toggle(isOn: Binding(
                    get: { selectedIndexes.contains(file.index) },
                    set: { isOn in
                        if isOn {
                            selectedIndexes.insert(file.index)
                        } else {
                            selectedIndexes.remove(file.index)
                        }
                    }
                )) {
                    VStack(alignment: .leading, spacing: 2) {
                        Text(file.path)
                            .lineLimit(2)
                        Text(Self.byteFormatter.string(fromByteCount: Int64(clamping: file.sizeBytes)))
                            .font(.caption)
                            .foregroundStyle(.secondary)
                    }
                }
            }

            Divider()

            HStack {
                Button("Cancel") {
                    onCancel()
                }
                .keyboardShortcut(.cancelAction)

                Spacer()

                Button("Download") {
                    onConfirm(Array(selectedIndexes))
                }
                .keyboardShortcut(.defaultAction)
                .disabled(selectedIndexes.isEmpty)
            }
            .padding()
        }
        .frame(minWidth: 480, minHeight: 320)
    }
}
