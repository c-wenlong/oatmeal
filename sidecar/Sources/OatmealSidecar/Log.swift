import Foundation

/// Diagnostics go to stderr so stdout stays pure newline-delimited protocol.
/// One stray `print` on stdout desyncs the Rust parser for the rest of the run.
enum Log {
    private static let lock = NSLock()

    static func write(_ level: String, _ message: String) {
        lock.lock()
        defer { lock.unlock() }
        FileHandle.standardError.write(Data("sidecar [\(level)] \(message)\n".utf8))
    }

    static func info(_ message: String) { write("info", message) }
    static func warn(_ message: String) { write("warn", message) }
    static func error(_ message: String) { write("error", message) }
}

enum CaptureError: Error, CustomStringConvertible {
    case noDisplay
    case micUnavailable
    case screenRecordingDenied
    case modelUnusable(String)

    var description: String {
        switch self {
        case .noDisplay:
            return "No display available for ScreenCaptureKit to attach to."
        case .micUnavailable:
            return "Microphone is unavailable (permission denied or no input device)."
        case .screenRecordingDenied:
            return "Screen Recording permission is required to capture system audio."
        case let .modelUnusable(reason):
            return reason
        }
    }
}
