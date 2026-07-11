#!/usr/bin/env bash
set -euo pipefail
exec xcrun simctl spawn booted "$@"
