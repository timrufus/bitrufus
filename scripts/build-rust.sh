#!/bin/bash
set -euo pipefail

# Ensure cargo is on PATH (Xcode strips the shell PATH)
export PATH="$HOME/.cargo/bin:/usr/local/bin:$PATH"

REPO_ROOT="${SRCROOT}"
GENERATED_DIR="${REPO_ROOT}/apps/TorrentApp/Generated"
ACTIVE_LIB_DIR="${REPO_ROOT}/target/active"

# Map Xcode CONFIGURATION to Cargo profile
case "${CONFIGURATION:-Debug}" in
    Release)
        PROFILE="release"
        CARGO_PROFILE_FLAG="--release"
        ;;
    *)
        PROFILE="debug"
        CARGO_PROFILE_FLAG=""
        ;;
esac

# Map Xcode NATIVE_ARCH to Rust target triple
case "${NATIVE_ARCH:-arm64}" in
    arm64)  RUST_TARGET="aarch64-apple-darwin" ;;
    x86_64) RUST_TARGET="x86_64-apple-darwin" ;;
    *)
        echo "error: unsupported NATIVE_ARCH=${NATIVE_ARCH:-unknown}"
        exit 1
        ;;
esac

TARGET_LIB_DIR="${REPO_ROOT}/target/${RUST_TARGET}/${PROFILE}"

mkdir -p "${GENERATED_DIR}"
mkdir -p "${ACTIVE_LIB_DIR}"

cd "${REPO_ROOT}"

# Build the Rust library for the active architecture
cargo build ${CARGO_PROFILE_FLAG} -p core --target "${RUST_TARGET}"

# Stage the static library at a fixed path so Xcode can link it regardless of arch/profile
cp "${TARGET_LIB_DIR}/libbitrufus_core.a" "${ACTIVE_LIB_DIR}/libbitrufus_core.a"

# Regenerate Swift bindings from the built dylib
cargo run --bin uniffi-bindgen -- generate \
    --library "${TARGET_LIB_DIR}/libbitrufus_core.dylib" \
    --language swift \
    --out-dir "${GENERATED_DIR}"
