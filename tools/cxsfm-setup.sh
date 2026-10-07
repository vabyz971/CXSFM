#!/usr/bin/env bash
# CXSFM layer setup — stage the Vulkan implicit layer once.
#
# Copies libcxsfm.so (which carries both the classic ctor entry AND the
# Vulkan layer exports) to a STABLE container-visible path and writes the
# loader manifest next to it. Run once (and after every rebuild):
#
#   ./tools/cxsfm-setup.sh
#
# Then add to the game's Steam launch options (set once, kept by Steam):
#   VK_ADD_LAYER_PATH="$HOME/.cache/cxsfm" VK_INSTANCE_LAYERS=VK_LAYER_CXSFM_overlay %command%
#
# Afterwards the game loads the framework itself at startup: no watchdog,
# no ptrace, no LD_PRELOAD needed for the render path.
set -euo pipefail

TOOLS_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(dirname "${TOOLS_DIR}")"
SRC_SO="${CXSF_SO:-${REPO_ROOT}/target/release/libcxsfm.so}"
CACHE_DIR="${XDG_CACHE_HOME:-${HOME}/.cache}/cxsfm"
LAYER_SO="${CACHE_DIR}/libcxsfm_layer.so"
MANIFEST="${CACHE_DIR}/VkLayer_CXSFM.json"

if [[ ! -f "${SRC_SO}" ]]; then
	echo "Framework .so not found at ${SRC_SO}." >&2
	echo "Build it first: cargo build --release" >&2
	exit 1
fi

mkdir -p "${CACHE_DIR}"
cp -f "${SRC_SO}" "${LAYER_SO}"
chmod 755 "${LAYER_SO}"

cat >"${MANIFEST}" <<EOF
{
  "file_format_version": "1.2.0",
  "layer": {
    "name": "VK_LAYER_CXSFM_overlay",
    "type": "GLOBAL",
    "library_path": "${LAYER_SO}",
    "api_version": "1.3.0",
    "implementation_version": "1",
    "description": "CXSFM in-game overlay framework",
    "functions": {
      "vkNegotiateLoaderLayerInterfaceVersion": "vkNegotiateLoaderLayerInterfaceVersion"
    }
  }
}
EOF
chmod 644 "${MANIFEST}" "${LAYER_SO}"

echo "Layer staged:"
echo "  library:  ${LAYER_SO}"
echo "  manifest: ${MANIFEST}"
echo
echo "Add to Steam launch options (once):"
echo "  VK_ADD_LAYER_PATH=\"\$HOME/.cache/cxsfm\" VK_INSTANCE_LAYERS=VK_LAYER_CXSFM_overlay %command%"
echo
echo "Verify the export exists:"
echo "  nm -D ${LAYER_SO} | grep vkNegotiate"
