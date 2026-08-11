import AVFoundation
import CoreGraphics
import Foundation
import ScreenCaptureKit
import SidecarProtocol

/// TCC checks for the two capabilities capture needs.
///
/// Both are attributed to the *host application bundle*, not to this binary — in
/// a release build that's Oatmeal.app; under `tauri dev` it's whichever terminal
/// launched the dev server. That's why the UI has to name the app being granted
/// rather than saying "grant Oatmeal".
enum Permissions {

    // MARK: Microphone

    static func microphone() -> PermissionState {
        switch AVCaptureDevice.authorizationStatus(for: .audio) {
        case .authorized: return .granted
        case .notDetermined: return .undetermined
        // `.restricted` (parental controls / MDM) is reported as denied: the user
        // can't fix it from here either way, and a third UI state buys nothing.
        case .denied, .restricted: return .denied
        @unknown default: return .denied
        }
    }

    /// Prompts for microphone access. Only does anything while `undetermined`;
    /// macOS silently no-ops a second prompt after a denial.
    static func requestMicrophone() async -> PermissionState {
        guard microphone() == .undetermined else { return microphone() }
        _ = await AVCaptureDevice.requestAccess(for: .audio)
        return microphone()
    }

    // MARK: Screen Recording (required for system audio)

    /// ScreenCaptureKit needs Screen Recording permission even in audio-only
    /// mode. There is no audio-only TCC scope to ask for instead.
    static func screenRecording() -> PermissionState {
        CGPreflightScreenCaptureAccess() ? .granted : .denied
    }

    /// Triggers the Screen Recording prompt.
    ///
    /// Unlike the mic, CoreGraphics gives no way to distinguish "never asked"
    /// from "denied", so this always attempts the request; when a denial is
    /// already recorded the call returns false without showing anything.
    static func requestScreenRecording() -> PermissionState {
        if CGPreflightScreenCaptureAccess() { return .granted }
        _ = CGRequestScreenCaptureAccess()
        return CGPreflightScreenCaptureAccess() ? .granted : .denied
    }

    /// True when the process is holding a stale Screen Recording denial.
    ///
    /// macOS hands a newly-granted Screen Recording capability only to freshly
    /// launched processes. A long-running app keeps failing to capture despite
    /// the checkbox being on, which reads as a broken app unless we say
    /// "relaunch". Detected by asking ScreenCaptureKit for real content: the
    /// preflight can disagree with what SCShareableContent will actually hand us.
    static func screenRecordingNeedsRelaunch() async -> Bool {
        guard CGPreflightScreenCaptureAccess() else { return false }
        do {
            let content = try await SCShareableContentBox.current()
            // With permission truly live we get at least one display.
            return content.isEmpty
        } catch {
            return true
        }
    }

    static func snapshot() async -> SidecarEvent {
        .permissions(
            microphone: microphone(),
            screenRecording: screenRecording(),
            needsRelaunch: await screenRecordingNeedsRelaunch())
    }

    /// Prompts, and reports where things stand afterwards.
    ///
    /// `pane` narrows it to one capability. Asking for both fires two system
    /// dialogs one after the other, which is right for a first-run "set me up"
    /// and wrong for a button sitting on one row.
    static func request(pane: String? = nil) async -> SidecarEvent {
        switch pane {
        case "microphone":
            _ = await requestMicrophone()
        case "screen_recording":
            _ = requestScreenRecording()
        default:
            _ = await requestMicrophone()
            _ = requestScreenRecording()
        }
        return await snapshot()
    }
}

/// Thin wrapper so `Permissions` doesn't need to know ScreenCaptureKit's shape.
enum SCShareableContentBox {
    /// Returns the available displays, or throws if the framework refuses.
    static func current() async throws -> [SCDisplay] {
        try await SCShareableContent.excludingDesktopWindows(
            false, onScreenWindowsOnly: false
        ).displays
    }
}
