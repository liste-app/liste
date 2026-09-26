// swift-tools-version: 6.0
// ListeKit: view models and platform-neutral code shared by the macOS and
// iOS apps. It links the core through the UniFFI-generated Swift bindings
// (ListeCore, generated into Sources/ListeCore by `just apple-bindings`)
// and the XCFramework in bindings/generated/apple. No third-party
// dependencies.

import PackageDescription

let package = Package(
    name: "ListeKit",
    platforms: [.macOS("26.0"), .iOS("26.0")],
    products: [
        .library(name: "ListeKit", targets: ["ListeKit"]),
        .library(name: "ListeCore", targets: ["ListeCore"]),
    ],
    targets: [
        .binaryTarget(
            name: "ListeCoreFFI",
            path: "../../../bindings/generated/apple/ListeCoreFFI.xcframework"
        ),
        // The generated bindings are Swift 5 mode: UniFFI's output is not
        // audited for strict concurrency, and it is regenerated on every
        // build.
        .target(
            name: "ListeCore",
            dependencies: ["ListeCoreFFI"],
            swiftSettings: [.swiftLanguageMode(.v5)]
        ),
        .target(
            name: "ListeKit",
            dependencies: ["ListeCore"],
            swiftSettings: [.swiftLanguageMode(.v6)]
        ),
        .testTarget(
            name: "ListeKitTests",
            dependencies: ["ListeKit"],
            swiftSettings: [.swiftLanguageMode(.v6)]
        ),
    ]
)
