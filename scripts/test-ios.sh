#!/usr/bin/env bash
set -euo pipefail

if [ ! -f Cargo.toml ]; then
    echo "run this from the midi-io repo root" >&2
    exit 1
fi

TARGET=aarch64-apple-ios-sim

if ! xcrun simctl list devices booted | grep -q "(Booted)"; then
    udid="${IOS_SIM_UDID:-$(xcrun simctl list devices available | grep -E 'iPhone' | grep -oE '[0-9A-Fa-f-]{36}' | head -n1)}"
    if [ -z "$udid" ]; then
        echo "No available iPhone simulator found" >&2
        exit 1
    fi
    echo "Booting simulator ${udid}"
    xcrun simctl boot "$udid"
    xcrun simctl bootstatus "$udid" -b
fi

booted="$(xcrun simctl list devices booted | grep "(Booted)" | head -n1 | sed 's/^[[:space:]]*//')"
echo "Using booted simulator: $booted"

echo "Running iOS unit suite on the simulator"
cargo test --lib --target "$TARGET" -- --nocapture
