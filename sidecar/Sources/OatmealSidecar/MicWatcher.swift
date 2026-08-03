import AppKit
import CoreAudio
import Foundation

/// Notices when another app starts using the microphone.
///
/// The trigger the user asked for by name — and the one that most needs a
/// leash. "Something is using the mic" is not "a meeting is starting":
/// dictation tools, voice memos and browser tabs all hold the input device, and
/// a popup every time one does would be worse than no detection at all. So this
/// only ever *reports*; whether a given app may trigger anything is a rule the
/// Rust side owns (G21/G23), and nothing fires without one.
///
/// Uses the audio process-object API (macOS 14.4+), which is the only supported
/// way to attribute input to a process without private API.
final class MicWatcher {
    /// How often the device list is polled.
    ///
    /// There is no notification for "a process started input", so this polls.
    /// Two seconds is under the time it takes someone to join a call and say
    /// hello, and cheap: the query is a couple of property reads per process.
    private let interval: TimeInterval = 2.0

    private let onChange: ([MicUser], [MicUser]) -> Void
    private var timer: DispatchSourceTimer?
    private let queue = DispatchQueue(label: "oatmeal.sidecar.micwatch")
    private var active: [pid_t: MicUser] = [:]

    /// Processes never reported, whatever they do with the microphone.
    ///
    /// Our own capture holds the input device for the whole of a recording;
    /// reporting it would have Oatmeal detecting itself and offering to record
    /// the meeting it is already recording.
    private let ignoredPids: Set<pid_t>

    init(ignoring pids: Set<pid_t>, onChange: @escaping ([MicUser], [MicUser]) -> Void) {
        self.ignoredPids = pids
        self.onChange = onChange
    }

    func start() {
        let timer = DispatchSource.makeTimerSource(queue: queue)
        timer.schedule(deadline: .now(), repeating: interval)
        timer.setEventHandler { [weak self] in self?.poll() }
        timer.resume()
        self.timer = timer
        Log.info("Mic watcher started.")
    }

    func stop() {
        timer?.cancel()
        timer = nil
        active = [:]
    }

    private func poll() {
        let current = Self.currentInputUsers(ignoring: ignoredPids)
        let currentByPid = Dictionary(uniqueKeysWithValues: current.map { ($0.pid, $0) })

        let started = current.filter { active[$0.pid] == nil }
        let stopped = active.values.filter { currentByPid[$0.pid] == nil }

        active = currentByPid
        if !started.isEmpty || !stopped.isEmpty {
            onChange(started, Array(stopped))
        }
    }

    /// Every process currently running audio input.
    ///
    /// Static and side-effect free so it can be called once from a test or a
    /// one-shot probe without standing a watcher up.
    static func currentInputUsers(ignoring ignoredPids: Set<pid_t> = []) -> [MicUser] {
        var address = AudioObjectPropertyAddress(
            mSelector: kAudioHardwarePropertyProcessObjectList,
            mScope: kAudioObjectPropertyScopeGlobal,
            mElement: kAudioObjectPropertyElementMain)

        var size: UInt32 = 0
        guard
            AudioObjectGetPropertyDataSize(
                AudioObjectID(kAudioObjectSystemObject), &address, 0, nil, &size) == noErr,
            size > 0
        else { return [] }

        var objects = [AudioObjectID](
            repeating: 0, count: Int(size) / MemoryLayout<AudioObjectID>.size)
        guard
            AudioObjectGetPropertyData(
                AudioObjectID(kAudioObjectSystemObject), &address, 0, nil, &size, &objects)
                == noErr
        else { return [] }

        return objects.compactMap { object in
            guard let pid = processId(of: object), !ignoredPids.contains(pid) else { return nil }
            guard isRunningInput(object) else { return nil }
            return MicUser(pid: pid)
        }
    }

    private static func processId(of object: AudioObjectID) -> pid_t? {
        var address = AudioObjectPropertyAddress(
            mSelector: kAudioProcessPropertyPID,
            mScope: kAudioObjectPropertyScopeGlobal,
            mElement: kAudioObjectPropertyElementMain)
        var pid: pid_t = 0
        var size = UInt32(MemoryLayout<pid_t>.size)
        guard AudioObjectGetPropertyData(object, &address, 0, nil, &size, &pid) == noErr else {
            return nil
        }
        return pid
    }

    private static func isRunningInput(_ object: AudioObjectID) -> Bool {
        var address = AudioObjectPropertyAddress(
            mSelector: kAudioProcessPropertyIsRunningInput,
            mScope: kAudioObjectPropertyScopeGlobal,
            mElement: kAudioObjectPropertyElementMain)
        var running: UInt32 = 0
        var size = UInt32(MemoryLayout<UInt32>.size)
        guard AudioObjectGetPropertyData(object, &address, 0, nil, &size, &running) == noErr
        else { return false }
        return running != 0
    }
}

/// An app holding the microphone.
struct MicUser {
    let pid: pid_t
    let bundleId: String?
    let name: String?

    init(pid: pid_t) {
        self.pid = pid
        if let app = NSRunningApplication(processIdentifier: pid),
            let identifier = app.bundleIdentifier
        {
            self.bundleId = identifier
            self.name = app.localizedName
        } else {
            // `NSRunningApplication` only knows about processes LaunchServices
            // registered. It returns nil for a helper — and browsers run audio
            // in one, so "Meet in Chrome" would otherwise be undetectable, which
            // is one of the cases this feature exists for.
            let resolved = MicUser.owningApp(of: pid)
            self.bundleId = resolved?.bundleId
            self.name = resolved?.name
        }
    }

    /// Resolves a pid to the app that owns it, via its executable path.
    ///
    /// Walks to the **outermost** enclosing `.app`, not the innermost. A Chrome
    /// helper lives at
    /// `…/Google Chrome.app/Contents/Frameworks/…/Google Chrome Helper.app/…`,
    /// and the innermost bundle is the helper — a rule written about it would
    /// read "Google Chrome Helper (Renderer)" and would not survive a Chrome
    /// update. The outermost is the app the user recognises.
    static func owningApp(of pid: pid_t) -> (bundleId: String, name: String?)? {
        var buffer = [CChar](repeating: 0, count: 4096)
        let length = proc_pidpath(pid, &buffer, UInt32(buffer.count))
        guard length > 0 else { return nil }
        let path = String(cString: buffer)

        // Split on ".app/" and keep the first, which is the outermost bundle.
        var components: [String] = []
        for part in path.split(separator: "/", omittingEmptySubsequences: false) {
            components.append(String(part))
            if part.hasSuffix(".app") {
                break
            }
        }
        guard components.last?.hasSuffix(".app") == true else { return nil }
        let bundlePath = components.joined(separator: "/")

        guard let bundle = Bundle(path: bundlePath),
            let identifier = bundle.bundleIdentifier
        else { return nil }

        let name =
            (bundle.object(forInfoDictionaryKey: "CFBundleDisplayName") as? String)
            ?? (bundle.object(forInfoDictionaryKey: "CFBundleName") as? String)
            ?? URL(fileURLWithPath: bundlePath).deletingPathExtension().lastPathComponent
        return (identifier, name)
    }

    init(pid: pid_t, bundleId: String?, name: String?) {
        self.pid = pid
        self.bundleId = bundleId
        self.name = name
    }

    /// Whether this is something a per-app rule can even be written about.
    ///
    /// A bundle identifier is the only stable handle: pids are per-launch and a
    /// localized name changes with the system language. A background daemon or
    /// a script holding the input device has neither, so it can never be turned
    /// into "always record when X starts" and is not worth interrupting anyone
    /// about.
    var isRuleable: Bool {
        guard let bundleId, !bundleId.isEmpty else { return false }
        return true
    }
}
