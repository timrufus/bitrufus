#!/bin/bash
set -euo pipefail

# Ensure cargo is on PATH (Xcode strips the shell PATH)
export PATH="$HOME/.cargo/bin:/usr/local/bin:$PATH"

REPO_ROOT="${SRCROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)}"
GENERATED_DIR="${REPO_ROOT}/apps/TorrentApp/Generated"
ACTIVE_LIB_DIR="${REPO_ROOT}/target/active"

# Map Xcode CONFIGURATION to Cargo profile
# Use a string, not an array: bash 3.2 (macOS /bin/bash) treats "${empty_array[@]}"
# as unbound variable under set -u, crashing every Debug build.
CARGO_RELEASE_FLAG=""
case "${CONFIGURATION:-Debug}" in
    Release)
        PROFILE="release"
        CARGO_RELEASE_FLAG="--release"
        ;;
    *)
        PROFILE="debug"
        ;;
esac

# Map Xcode CURRENT_ARCH to Rust target triple (CURRENT_ARCH is the arch being built, not the host)
case "${CURRENT_ARCH:-arm64}" in
    arm64)  RUST_TARGET="aarch64-apple-darwin" ;;
    x86_64) RUST_TARGET="x86_64-apple-darwin" ;;
    *)
        echo "error: unsupported CURRENT_ARCH=${CURRENT_ARCH:-unknown}" >&2
        exit 1
        ;;
esac

TARGET_LIB_DIR="${REPO_ROOT}/target/${RUST_TARGET}/${PROFILE}"

mkdir -p "${GENERATED_DIR}"
mkdir -p "${ACTIVE_LIB_DIR}"

cd "${REPO_ROOT}"

# Build the Rust library for the active architecture
# shellcheck disable=SC2086  # word splitting intentional: flag is either "" or "--release"
cargo build ${CARGO_RELEASE_FLAG} -p bitrufus_core --target "${RUST_TARGET}"

# Stage the static library at a fixed path so Xcode can link it regardless of arch/profile
cp "${TARGET_LIB_DIR}/libbitrufus_core.a" "${ACTIVE_LIB_DIR}/libbitrufus_core.a"

# Regenerate Swift bindings — the host uniffi-bindgen binary is compiled without --release
# (fast incremental build); it reads the already-built dylib for introspection.
cargo run --bin uniffi-bindgen -- generate \
    --library "${TARGET_LIB_DIR}/libbitrufus_core.dylib" \
    --language swift \
    --out-dir "${GENERATED_DIR}"
