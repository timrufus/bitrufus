import SwiftUI

struct VolumeInfo: Identifiable {
    let id: URL
    let name: String
    let free: Int64
    let total: Int64
    let isExternal: Bool

    var usedFraction: Double {
        guard total > 0 else { return 0 }
        return Double(total - free) / Double(total)
    }
}

private func loadVolumes() -> [VolumeInfo] {
    let keys: [URLResourceKey] = [
        .volumeNameKey,
        .volumeIsRemovableKey,
        .volumeIsInternalKey,
        .volumeIsReadOnlyKey,
        .volumeTotalCapacityKey,
    ]
    guard let urls = FileManager.default.mountedVolumeURLs(
        includingResourceValuesForKeys: keys,
        options: .skipHiddenVolumes
    ) else { return [] }

    var result: [VolumeInfo] = []
    for url in urls {
        guard let vals = try? url.resourceValues(forKeys: Set(keys)),
              vals.volumeIsReadOnly != true,
              let total = vals.volumeTotalCapacity,
              total > 0
        else { continue }

        let attrs = try? FileManager.default.attributesOfFileSystem(forPath: url.path)
        let free = (attrs?[.systemFreeSize] as? Int) ?? 0
        let name = vals.volumeName ?? url.lastPathComponent
        let isInternal = vals.volumeIsInternal ?? true

        result.append(VolumeInfo(
            id: url,
            name: name,
            free: Int64(free),
            total: Int64(total),
            isExternal: !isInternal
        ))
    }
    return result.sorted { !$0.isExternal && $1.isExternal }
}

struct DiskSpacePopover: View {
    @State private var volumes: [VolumeInfo] = []

    private static let fmt: ByteCountFormatter = {
        let f = ByteCountFormatter()
        f.countStyle = .file
        return f
    }()

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text("Disk Space")
                .font(.headline)
                .padding(.bottom, 2)

            if volumes.isEmpty {
                Text("No volumes found")
                    .foregroundStyle(.secondary)
            } else {
                ForEach(volumes) { vol in
                    VolumeRow(vol: vol, fmt: Self.fmt)
                }
            }
        }
        .padding()
        .frame(minWidth: 280)
        .onAppear { volumes = loadVolumes() }
    }
}

private struct VolumeRow: View {
    let vol: VolumeInfo
    let fmt: ByteCountFormatter

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            HStack {
                Image(systemName: vol.isExternal ? "externaldrive.fill" : "internaldrive.fill")
                    .foregroundStyle(vol.isExternal ? .orange : .secondary)
                Text(vol.name)
                    .fontWeight(.medium)
                Spacer()
                Text("\(fmt.string(fromByteCount: vol.free)) free")
                    .foregroundStyle(.secondary)
                    .font(.caption)
            }
            ProgressView(value: vol.usedFraction)
                .tint(vol.usedFraction > 0.9 ? .red : vol.usedFraction > 0.75 ? .orange : .blue)
            Text("\(fmt.string(fromByteCount: vol.total - vol.free)) used of \(fmt.string(fromByteCount: vol.total))")
                .font(.caption)
                .foregroundStyle(.secondary)
        }
    }
}
