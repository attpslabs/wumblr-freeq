#!/bin/bash
set -euo pipefail

# Build the Rust SDK for iOS targets and generate Swift bindings.
# Run from the repo root: ./freeq-ios/build-rust.sh

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$REPO_ROOT"

# Prefer rustup-managed cargo/rustc over Homebrew (which lacks iOS targets)
export PATH="$HOME/.cargo/bin:$PATH"
# If Homebrew rustc still wins, force rustup's rustc
if rustc --print sysroot 2>/dev/null | grep -q Cellar; then
    for tc in "$HOME/.rustup"/toolchains/stable-*; do
        if [ -x "$tc/bin/rustc" ]; then
            export RUSTC="$tc/bin/rustc"
            echo "==> Overriding Homebrew rustc with: $RUSTC"
            break
        fi
    done
fi

export DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer

FEATURES="--features av"

# Whether to build the simulator slice WITH the AV feature. `openh264-sys2`
# (transitively pulled in by iroh-live's h264 feature) panics on the
# `aarch64-apple-ios-sim` target env ("Unknown target env: sim"), so the AV
# feature must be off for the sim slice. BUILD_SIM_AV=1 tries anyway.
#
# BUILD_SIM=1 (the new default) builds an AV-stub simulator slice — every
# FreeqAv method returns NotConnected/no-op. This is enough to let the
# XCTest target link and run unit tests on Simulator destinations; real
# AV behavior is only exercised on-device.
BUILD_SIM="${BUILD_SIM:-1}"
BUILD_SIM_AV="${BUILD_SIM_AV:-0}"

echo "==> Building for iOS device (aarch64-apple-ios)..."
IPHONEOS_DEPLOYMENT_TARGET=18.0 cargo rustc -p freeq-sdk-ffi $FEATURES --release --target aarch64-apple-ios --lib --crate-type staticlib

if [ "$BUILD_SIM" = "1" ]; then
    if [ "$BUILD_SIM_AV" = "1" ]; then
        echo "==> Building for iOS simulator WITH AV (aarch64-apple-ios-sim)..."
        IPHONEOS_DEPLOYMENT_TARGET=18.0 cargo rustc -p freeq-sdk-ffi $FEATURES --release --target aarch64-apple-ios-sim --lib --crate-type staticlib
    else
        echo "==> Building for iOS simulator WITHOUT AV (aarch64-apple-ios-sim) — stub FreeqAv lets the test target link"
        IPHONEOS_DEPLOYMENT_TARGET=18.0 cargo rustc -p freeq-sdk-ffi --release --target aarch64-apple-ios-sim --lib --crate-type staticlib
    fi
else
    echo "==> Skipping simulator slice (BUILD_SIM=0)"
fi

echo "==> Building host binary for bindgen..."
cargo build -p freeq-sdk-ffi $FEATURES --lib --release
cargo build -p freeq-sdk-ffi --bin uniffi-bindgen

echo "==> Generating Swift bindings..."
cargo run -p freeq-sdk-ffi --bin uniffi-bindgen -- generate \
    --library target/release/libfreeq_sdk_ffi.dylib \
    --language swift \
    --out-dir freeq-ios/Generated

echo "==> Assembling xcframework..."
rm -rf freeq-ios/FreeqSDK.xcframework
mkdir -p freeq-ios/FreeqSDK.xcframework/ios-arm64/Headers

# Headers
cp freeq-ios/Generated/freeqFFI.h freeq-ios/FreeqSDK.xcframework/ios-arm64/Headers/
cp freeq-ios/Generated/freeqFFI.modulemap freeq-ios/FreeqSDK.xcframework/ios-arm64/Headers/module.modulemap

# Static libs
cp target/aarch64-apple-ios/release/libfreeq_sdk_ffi.a freeq-ios/FreeqSDK.xcframework/ios-arm64/

if [ "$BUILD_SIM" = "1" ]; then
    mkdir -p freeq-ios/FreeqSDK.xcframework/ios-arm64-simulator/Headers
    cp freeq-ios/Generated/freeqFFI.h freeq-ios/FreeqSDK.xcframework/ios-arm64-simulator/Headers/
    cp freeq-ios/Generated/freeqFFI.modulemap freeq-ios/FreeqSDK.xcframework/ios-arm64-simulator/Headers/module.modulemap
    cp target/aarch64-apple-ios-sim/release/libfreeq_sdk_ffi.a freeq-ios/FreeqSDK.xcframework/ios-arm64-simulator/
fi

# Info.plist — listing only the slices we actually built.
if [ "$BUILD_SIM" = "1" ]; then
cat > freeq-ios/FreeqSDK.xcframework/Info.plist << 'EOF'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>AvailableLibraries</key>
	<array>
		<dict>
			<key>HeadersPath</key>
			<string>Headers</string>
			<key>LibraryIdentifier</key>
			<string>ios-arm64</string>
			<key>LibraryPath</key>
			<string>libfreeq_sdk_ffi.a</string>
			<key>SupportedArchitectures</key>
			<array>
				<string>arm64</string>
			</array>
			<key>SupportedPlatform</key>
			<string>ios</string>
		</dict>
		<dict>
			<key>HeadersPath</key>
			<string>Headers</string>
			<key>LibraryIdentifier</key>
			<string>ios-arm64-simulator</string>
			<key>LibraryPath</key>
			<string>libfreeq_sdk_ffi.a</string>
			<key>SupportedArchitectures</key>
			<array>
				<string>arm64</string>
			</array>
			<key>SupportedPlatform</key>
			<string>ios</string>
			<key>SupportedPlatformVariant</key>
			<string>simulator</string>
		</dict>
	</array>
	<key>CFBundlePackageType</key>
	<string>XFWK</string>
	<key>XCFrameworkFormatVersion</key>
	<string>1.0</string>
</dict>
</plist>
EOF
else
cat > freeq-ios/FreeqSDK.xcframework/Info.plist << 'EOF'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>AvailableLibraries</key>
	<array>
		<dict>
			<key>HeadersPath</key>
			<string>Headers</string>
			<key>LibraryIdentifier</key>
			<string>ios-arm64</string>
			<key>LibraryPath</key>
			<string>libfreeq_sdk_ffi.a</string>
			<key>SupportedArchitectures</key>
			<array>
				<string>arm64</string>
			</array>
			<key>SupportedPlatform</key>
			<string>ios</string>
		</dict>
	</array>
	<key>CFBundlePackageType</key>
	<string>XFWK</string>
	<key>XCFrameworkFormatVersion</key>
	<string>1.0</string>
</dict>
</plist>
EOF
fi

echo "==> Done! xcframework at freeq-ios/FreeqSDK.xcframework"
echo "    Swift bindings at freeq-ios/Generated/freeq.swift"
