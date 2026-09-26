#!/usr/bin/env bash
# Build the core for macOS (arm64 and x86_64), generate the Swift bindings,
# and package the static library as an XCFramework the Xcode project links.
# Output goes to bindings/generated/apple/ (ignored by git) and the
# generated Swift file is copied into the ListeKit package. iOS device and
# simulator slices are added here when the iOS target is built.
set -euo pipefail
cd "$(dirname "$0")/.."

if [ -z "${DEVELOPER_DIR:-}" ] && [ -d /Applications/Xcode.app/Contents/Developer ]; then
    export DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer
fi

OUT=bindings/generated/apple
TARGETS=(aarch64-apple-darwin x86_64-apple-darwin)
PROFILE="${PROFILE:-release}"
# The app's deployment target (current macOS minus one major version); the
# Rust objects must not claim a newer one than the app links for.
export MACOSX_DEPLOYMENT_TARGET="${MACOSX_DEPLOYMENT_TARGET:-26.0}"

for t in "${TARGETS[@]}"; do
    rustup target add "$t" >/dev/null 2>&1 || true
done
cargo build -p liste-bindings --"$PROFILE" $(printf -- '--target %s ' "${TARGETS[@]}")

rm -rf "$OUT"
mkdir -p "$OUT/swift" "$OUT/headers"

# Swift bindings from the dylib's embedded metadata.
cargo run -q -p liste-bindings --features cli --bin uniffi-bindgen -- generate \
    --library "target/${TARGETS[0]}/$PROFILE/libliste_bindings.dylib" \
    --language swift --config bindings/uniffi.toml --out-dir "$OUT/swift"

# One universal static library.
lipo -create $(printf -- "target/%s/$PROFILE/libliste_bindings.a " "${TARGETS[@]}") \
    -output "$OUT/libliste_bindings.a"

cp "$OUT/swift/ListeCoreFFI.h" "$OUT/headers/"
cp "$OUT/swift/ListeCoreFFI.modulemap" "$OUT/headers/module.modulemap"
xcodebuild -create-xcframework \
    -library "$OUT/libliste_bindings.a" -headers "$OUT/headers" \
    -output "$OUT/ListeCoreFFI.xcframework" >/dev/null

mkdir -p apps/apple/ListeKit/Sources/ListeCore
cp "$OUT/swift/ListeCore.swift" apps/apple/ListeKit/Sources/ListeCore/ListeCore.swift

echo "XCFramework: $OUT/ListeCoreFFI.xcframework ($(du -sh "$OUT/ListeCoreFFI.xcframework" | cut -f1))"
echo "Swift bindings: apps/apple/ListeKit/Sources/ListeCore/ListeCore.swift"
