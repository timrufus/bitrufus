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

# Map Xcode CURRENT_ARCH to Rust target triple.
# CURRENT_ARCH is "undefined_arch" when the Run Script phase isn't per-arch
# (the default for target-level scripts). Fall back to ARCHS if it's a single
# value; if ARCHS has multiple values (universal/archive build), build each
# arch separately and lipo them into a fat binary; otherwise fall back to
# the host arch via `uname -m`.
ARCH="${CURRENT_ARCH:-}"
if [ -z "${ARCH}" ] || [ "${ARCH}" = "undefined_arch" ]; then
    ARCH_COUNT="$(echo "${ARCHS:-}" | wc -w | tr -d ' ')"
    if [ -n "${ARCHS:-}" ] && [ "${ARCH_COUNT}" = "1" ]; then
        ARCH="${ARCHS}"
    elif [ -n "${ARCHS:-}" ] && [ "${ARCH_COUNT}" -gt "1" ]; then
        # Universal (multi-arch) build: compile each slice then lipo together.
        # Use strings, not arrays: bash 3.2 under set -u treats empty array
        # expansion as an unbound variable error.
        mkdir -p "${GENERATED_DIR}" "${ACTIVE_LIB_DIR}"
        cd "${REPO_ROOT}"
        SLICE_LIBS=""
        for a in ${ARCHS}; do
            case "${a}" in
                arm64)  t="aarch64-apple-darwin" ;;
                x86_64) t="x86_64-apple-darwin" ;;
                *) echo "error: unsupported arch=${a} in ARCHS=${ARCHS}" >&2; exit 1 ;;
            esac
            # shellcheck disable=SC2086
            cargo build ${CARGO_RELEASE_FLAG} -p bitrufus_core --target "${t}"
            SLICE_LIBS="${SLICE_LIBS} ${REPO_ROOT}/target/${t}/${PROFILE}/libbitrufus_core.a"
        done
        # shellcheck disable=SC2086  # word splitting intentional for SLICE_LIBS
        lipo -create ${SLICE_LIBS} -output "${ACTIVE_LIB_DIR}/libbitrufus_core.a"
        # uniffi-bindgen dlopen()s the dylib, so use the host-compatible slice,
        # not the first iterated slice (which may be a different arch).
        case "$(uname -m)" in
            arm64)  HOST_TARGET="aarch64-apple-darwin" ;;
            x86_64) HOST_TARGET="x86_64-apple-darwin" ;;
            *) echo "error: unsupported host arch=$(uname -m)" >&2; exit 1 ;;
        esac
        cargo run --bin uniffi-bindgen -- generate \
            --library "${REPO_ROOT}/target/${HOST_TARGET}/${PROFILE}/libbitrufus_core.dylib" \
            --language swift \
            --out-dir "${GENERATED_DIR}"
        exit 0
    else
        case "$(uname -m)" in
            arm64)  ARCH="arm64" ;;
            x86_64) ARCH="x86_64" ;;
            *) echo "error: unsupported host arch=$(uname -m)" >&2; exit 1 ;;
        esac
    fi
fi

case "${ARCH}" in
    arm64)  RUST_TARGET="aarch64-apple-darwin" ;;
    x86_64) RUST_TARGET="x86_64-apple-darwin" ;;
    *)
        echo "error: unsupported arch=${ARCH:-unknown} (CURRENT_ARCH=${CURRENT_ARCH:-unset} ARCHS=${ARCHS:-unset})" >&2
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
