#!/usr/bin/env bash
set -euo pipefail

KERNEL_VERSION=6.18.5
KSOURCE="https://cdn.kernel.org/pub/linux/kernel/v6.x/linux-${KERNEL_VERSION}.tar.xz"
KSOURCE_SHA256="189d1f409cef8d0d234210e04595172df392f8cb297e14b447ed95720e2fd940"
KIMAGE="midi-io-kernel-build:1"

DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${DIR}/../.." && pwd)"
FRAGMENT="${1:-alsa-seq}"
OUT_DIR="${2:-${REPO_ROOT}/target/midi-io-kernel}"
OUTPUT_NAME="${KERNEL_VERSION}-${FRAGMENT}.Image"
SOURCE_FILE="${OUT_DIR}/source-${KERNEL_VERSION}.tar.xz"

test -f "${DIR}/fragments/${FRAGMENT}.conf" || {
  echo "no such fragment: ${DIR}/fragments/${FRAGMENT}.conf" >&2
  echo "available: $(cd "${DIR}/fragments" && ls *.conf 2>/dev/null | sed 's/\.conf//')" >&2
  exit 1
}

mkdir -p "${OUT_DIR}"

echo "==> toolchain image"
container build "${DIR}" -f "${DIR}/Dockerfile" -t "${KIMAGE}"

if [ ! -f "${SOURCE_FILE}" ]; then
  echo "==> fetching kernel ${KERNEL_VERSION} source"
  curl -SsL -o "${SOURCE_FILE}" "${KSOURCE}"
fi
echo "${KSOURCE_SHA256}  ${SOURCE_FILE}" | shasum -a 256 -c - || {
  echo "checksum mismatch for ${SOURCE_FILE}; delete it and re-run" >&2
  exit 1
}

echo "==> building kernel (fragment: ${FRAGMENT})"
container run --cpus 8 --rm --memory 16g \
  -v "${DIR}:/kernel" \
  -v "${OUT_DIR}:/kernel-cache" \
  --env FRAGMENT="${FRAGMENT}" \
  --env SOURCE_NAME="$(basename "${SOURCE_FILE}")" \
  --env OUTPUT_NAME="${OUTPUT_NAME}" \
  --env LOCALVERSION="-${FRAGMENT}" \
  --cwd /kernel \
  "${KIMAGE}" \
  /bin/bash -c "./build-in-container.sh"

echo
echo "==> done: ${OUT_DIR}/${OUTPUT_NAME}"
echo "    use it:  container run -k ${OUT_DIR}/${OUTPUT_NAME} <image> ..."
