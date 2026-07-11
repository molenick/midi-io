#!/usr/bin/env bash
set -euo pipefail

DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${DIR}/.." && pwd)"
MODE="${1:-test}"

TEST_IMAGE="midi-io-linux-test:1"
KERNEL_FRAGMENT="alsa-seq"
KERNEL_CACHE_DIR="${REPO_ROOT}/target/midi-io-kernel"
KERNEL_IMAGE="${KERNEL_CACHE_DIR}/6.18.5-${KERNEL_FRAGMENT}.Image"

CACHE_DIR="${MIDI_IO_CONTAINER_CACHE:-$HOME/.cache/midi-io-container}"
mkdir -p "${CACHE_DIR}/target" "${CACHE_DIR}/cargo-registry"
CACHE_MOUNTS=(
  -v "${CACHE_DIR}/target:/tmp/cargo-target"
  -v "${CACHE_DIR}/cargo-registry:/usr/local/cargo/registry"
)

if ! container image inspect "${TEST_IMAGE}" >/dev/null 2>&1; then
  echo "==> test image"
  container build "${DIR}" -f "${DIR}/test-container.Dockerfile" -t "${TEST_IMAGE}"
fi

if [ "${MODE}" = "check" ]; then
  echo "==> clippy + build (no kernel)"
  container run --rm \
    -v "${REPO_ROOT}:/workspace" \
    "${CACHE_MOUNTS[@]}" \
    --cwd /workspace \
    "${TEST_IMAGE}" \
    bash -c '
      set -e
      export CARGO_TARGET_DIR=/tmp/cargo-target
      cargo clippy --all-targets -- -D warnings
      cargo build --tests
    '
  exit 0
fi

if [ ! -f "${KERNEL_IMAGE}" ]; then
  echo "==> custom kernel not found, building it"
  "${DIR}/kernel/build-kernel.sh" "${KERNEL_FRAGMENT}" "${KERNEL_CACHE_DIR}"
fi

echo "==> clippy + build + cargo test on custom kernel ($(basename "${KERNEL_IMAGE}"))"
container run --rm \
  -k "${KERNEL_IMAGE}" \
  -v "${REPO_ROOT}:/workspace" \
  "${CACHE_MOUNTS[@]}" \
  --cwd /workspace \
  "${TEST_IMAGE}" \
  bash -c '
    set -e
    echo "kernel: $(uname -r)"
    test -e /dev/snd/seq || { echo "FATAL: /dev/snd/seq missing - wrong kernel?" >&2; exit 1; }
    export CARGO_TARGET_DIR=/tmp/cargo-target
    cargo clippy --all-targets -- -D warnings
    cargo test -- --include-ignored
  '
