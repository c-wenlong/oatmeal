// swift-tools-version: 6.0
import PackageDescription

// The Oatmeal sidecar. Owns everything that must be Swift: ScreenCaptureKit,
// AVFoundation, WhisperKit. Talks to the Rust core over newline-delimited JSON
// on stdio — audio bytes never cross the boundary, only transcript events.
//
// At G4 this emits scripted fake events and has no audio dependencies at all.
// That is deliberate: it decouples the IPC/supervision problem from the audio
// problem, so when G6 lands real capture, any breakage is unambiguously audio.
let package = Package(
    name: "OatmealSidecar",
    platforms: [.macOS(.v14)],
    dependencies: [
        .package(url: "https://github.com/argmaxinc/WhisperKit.git", from: "0.9.0")
    ],
    targets: [
        .executableTarget(
            name: "OatmealSidecar",
            dependencies: [
                "SidecarProtocol", "SidecarCore",
                .product(name: "WhisperKit", package: "WhisperKit"),
            ],
            swiftSettings: [.swiftLanguageMode(.v5)]
        ),
        .target(
            name: "SidecarProtocol",
            swiftSettings: [.swiftLanguageMode(.v5)]
        ),
        // Audio logic with no AVFoundation/ScreenCaptureKit dependency, so the
        // fiddly parts (ring buffer wrap-around, stream alignment, silence
        // detection) are testable without a microphone or a display.
        .target(
            name: "SidecarCore",
            swiftSettings: [.swiftLanguageMode(.v5)]
        ),
        .testTarget(
            name: "SidecarProtocolTests",
            dependencies: ["SidecarProtocol"],
            swiftSettings: [.swiftLanguageMode(.v5)]
        ),
        .testTarget(
            name: "SidecarCoreTests",
            dependencies: ["SidecarCore"],
            swiftSettings: [.swiftLanguageMode(.v5)]
        ),
    ]
)
