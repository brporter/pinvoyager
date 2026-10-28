// swift-tools-version: 6.0
import PackageDescription

let package = Package(
  name: "PinVoyagerKit",
  platforms: [.iOS(.v18), .macOS(.v15)],
  products: [
    .library(name: "PinVoyagerDomain", targets: ["PinVoyagerDomain"]),
    .library(name: "PinVoyagerData", targets: ["PinVoyagerData"]),
  ],
  targets: [
    .target(name: "PinVoyagerDomain"),
    .target(name: "PinVoyagerData", dependencies: ["PinVoyagerDomain"]),
    .testTarget(name: "PinVoyagerDomainTests", dependencies: ["PinVoyagerDomain"]),
    .testTarget(name: "PinVoyagerDataTests", dependencies: ["PinVoyagerData"]),
  ]
)
