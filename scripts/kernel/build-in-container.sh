#!/bin/bash
set -euo pipefail

FRAGMENT="${FRAGMENT:?FRAGMENT env (fragment name, no extension) is required}"
KARCH=arm64
CROSS_COMPILE=aarch64-linux-gnu-
IMAGE_PATH=arch/arm64/boot/Image
FRAG_FILE="/kernel/fragments/${FRAGMENT}.conf"
SRC_FILE="/kernel-cache/${SOURCE_NAME:?SOURCE_NAME env is required}"
OUT_FILE="/kernel-cache/${OUTPUT_NAME:?OUTPUT_NAME env is required}"

test -f "${FRAG_FILE}" || { echo "no such fragment: ${FRAG_FILE}" >&2; exit 1; }

mkdir -p /kbuild
tar -xf "${SRC_FILE}" -C /kbuild --strip-components=1
cp /kernel/base/config-arm64 /kbuild/.config

cd /kbuild
./scripts/kconfig/merge_config.sh -m -O . .config "${FRAG_FILE}"
make ARCH="${KARCH}" CROSS_COMPILE="${CROSS_COMPILE}" olddefconfig

fail=0
while IFS= read -r line; do
  case "${line}" in ''|\#*) continue ;; esac
  sym="${line%%=*}"
  if ! grep -qx -- "${line}" .config; then
    echo "FRAGMENT VALIDATION: requested '${line}' but got '$(grep -- "^${sym}=" .config || echo "${sym} unset")'" >&2
    fail=1
  fi
done < "${FRAG_FILE}"
[ "${fail}" -eq 0 ] || { echo "Fragment validation failed: symbols dropped by olddefconfig (unmet deps?)" >&2; exit 1; }
echo "Fragment validation OK: all '${FRAGMENT}' symbols present in final .config"

make ARCH="${KARCH}" CROSS_COMPILE="${CROSS_COMPILE}" -j"$(( $(nproc) - 1 ))" LOCALVERSION="${LOCALVERSION:-}"
cp "${IMAGE_PATH}" "${OUT_FILE}"
echo "wrote ${OUT_FILE}"
