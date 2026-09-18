import Foundation

// Shared formatters, so sizes render identically in every view.
enum Formatters {
    static let bytes: ByteCountFormatter = {
        let f = ByteCountFormatter()
        f.countStyle = .file
        return f
    }()
}
