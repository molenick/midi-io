#!/usr/bin/env bash
set -euo pipefail

DC_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$DC_DIR/.." && pwd)"

PLATFORM="${1:-all}"
MODE="${2:-check}"

deny_check() {
    if ! command -v cargo-deny >/dev/null 2>&1; then
        echo "cargo-deny not installed - run 'cargo install cargo-deny' (deny check FAILED)." >&2
        return 1
    fi
    cargo deny check
}

full_matrix() {
    if [ "$(uname -s)" != "Darwin" ]; then
        echo "The full matrix drives macOS + iOS + Linux and must run on macOS." >&2
        exit 1
    fi

    cd "$REPO_ROOT"

    if ! rustup toolchain list | grep -q '^nightly'; then
        echo "Installing nightly toolchain for rustfmt"
        rustup toolchain install nightly --profile minimal --component rustfmt
    elif ! rustup component list --toolchain nightly --installed 2>/dev/null | grep -q '^rustfmt'; then
        echo "Adding rustfmt to nightly"
        rustup component add rustfmt --toolchain nightly
    fi

    local failed=0
    run() { "$@" || failed=1; }

    run cargo +nightly fmt --check
    run deny_check
    run cargo clippy --all-targets -- -D warnings
    run cargo test -- --include-ignored
    run cargo clippy --target aarch64-apple-ios-sim --all-targets -- -D warnings
    run bash "$DC_DIR/test-ios.sh"
    run bash "$DC_DIR/test-container.sh" test

    return $failed
}

case "$PLATFORM" in
    all|check|test)
        full_matrix
        exit $?
        ;;
    -h|--help|help)
        usage
        ;;
esac

case "$PLATFORM" in
    macos)
        cargo clippy --all-targets -- -D warnings
        if [ "$MODE" = "test" ]; then
            cargo test -- --include-ignored
        fi
        ;;
    linux)
        bash "$DC_DIR/test-container.sh" "$MODE"
        ;;
    ios)
        cargo clippy --target aarch64-apple-ios-sim --all-targets -- -D warnings
        if [ "$MODE" = "test" ]; then
            (cd "$REPO_ROOT" && bash "$DC_DIR/test-ios.sh")
        fi
        ;;
    *)
esac
