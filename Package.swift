// swift-tools-version: 6.0
import PackageDescription

let package = Package(
    name: "Nexus",
    platforms: [.macOS("26.0")],
    products: [
        .executable(name: "Nexus", targets: ["Nexus"]),
    ],
    targets: [
        .executableTarget(
            name: "Nexus",
            path: "swift/Sources/ClipKeep",
            swiftSettings: [.swiftLanguageMode(.v5)]
        ),
        .testTarget(
            name: "NexusTests",
            dependencies: ["Nexus"],
            path: "swift/Tests/ClipKeepTests"
        ),
    ]
)
