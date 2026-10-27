// swift-tools-version: 6.0
import PackageDescription

let package = Package(
    name: "Spiritglass",
    platforms: [.macOS(.v14)],
    products: [.executable(name: "Spiritglass", targets: ["Spiritglass"])],
    targets: [
        .target(name: "SpiritglassCore"),
        .executableTarget(name: "Spiritglass", dependencies: ["SpiritglassCore"]),
        .testTarget(name: "SpiritglassCoreTests", dependencies: ["SpiritglassCore"]),
    ]
)
