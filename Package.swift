// swift-tools-version: 6.0
import PackageDescription

let package = Package(
    name: "ClipKeep",
    platforms: [.macOS("26.0")],
    products: [
        .executable(name: "ClipKeep", targets: ["ClipKeep"]),
    ],
    targets: [
        .executableTarget(
            name: "ClipKeep",
            path: "swift/Sources/ClipKeep",
            swiftSettings: [.swiftLanguageMode(.v5)]
        ),
    ]
)
