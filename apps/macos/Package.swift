// swift-tools-version: 6.0
import PackageDescription

// Build the Rust engine first: scripts/build-core.sh. It creates
// Frameworks/LantaranFFI.xcframework and Sources/LantaranCore/lantaran_ffi.swift.
let package = Package(
    name: "Lantaran",
    platforms: [.macOS(.v14)],
    targets: [
        .binaryTarget(name: "LantaranFFI", path: "Frameworks/LantaranFFI.xcframework"),
        .target(
            name: "LantaranCore",
            dependencies: ["LantaranFFI"],
            path: "Sources/LantaranCore",
            swiftSettings: [.swiftLanguageMode(.v5)],
            linkerSettings: [
                .linkedFramework("CoreFoundation"),
                .linkedFramework("Security"),
                .linkedFramework("SystemConfiguration"),
            ]
        ),
        .executableTarget(
            name: "Lantaran",
            dependencies: ["LantaranCore"],
            path: "Sources/Lantaran",
            swiftSettings: [.swiftLanguageMode(.v5)]
        ),
        .testTarget(
            name: "LantaranTests",
            dependencies: ["Lantaran"],
            path: "Tests/LantaranTests",
            swiftSettings: [.swiftLanguageMode(.v5)]
        ),
    ]
)
