#!/usr/bin/env bash
# CXSFM hang dump — snapshot where every game thread is stuck.
#
# Run it WHILE the game is frozen (black screen after Quit, ~1 fps…),
# before killing it. Always records each thread's name, state and
# kernel wait channel (/proc, no ptrace needed). When gdb is available
# and ptrace is allowed, also records every thread's backtrace (the
# release .so is stripped: frames show module + offset, libc/Unity
# exports by name — enough to tell cxsfm from the engine).
#
# Usage:
#   ./tools/cxsfm-hang-dump.sh            # finds CarX_Street.x86_64
#   ./tools/cxsfm-hang-dump.sh <pid>
#   sudo -E nix shell nixpkgs#gdb -c ./tools/cxsfm-hang-dump.sh <pid>
#     (backtraces: Yama ptrace_scope=1 forbids attaching to a process
#     that is not our child without root)
# Output: ~/.cache/cxsfm/hang-<date>.txt (path printed at the end).
set -euo pipefail

PID="${1:-$(pgrep -n -f 'CarX_Street\.x86_64' || true)}"
if [[ -z "${PID}" || ! -d "/proc/${PID}" ]]; then
	echo "game process not found (pass the pid explicitly)" >&2
	exit 1
fi

OUT_DIR="${HOME}/.cache/cxsfm"
mkdir -p "${OUT_DIR}"
OUT="${OUT_DIR}/hang-$(date +%Y%m%d-%H%M%S).txt"

{
	echo "# pid ${PID} — $(tr '\0' ' ' <"/proc/${PID}/cmdline" | cut -c1-200)"
	echo "# $(date -Is)"
	echo
	echo "## threads: tid name state wchan"
	for t in /proc/"${PID}"/task/*; do
		tid="${t##*/}"
		name="$(cat "${t}/comm" 2>/dev/null || echo '?')"
		state="$(awk '{print $3}' "${t}/stat" 2>/dev/null || echo '?')"
		wchan="$(cat "${t}/wchan" 2>/dev/null || echo '?')"
		printf '%-8s %-18s %-2s %s\n' "${tid}" "${name}" "${state}" "${wchan}"
	done
	echo
	echo "## modules (base of each mapped .so, for raw addresses)"
	awk '$6 ~ /\.so/ && !seen[$6]++ {print $1, $6}' "/proc/${PID}/maps" 2>/dev/null |
		grep -E 'cxsfm|GameAssembly|UnityPlayer|libX11|libSDL|libvulkan|libc\.so' || true
	echo
	if command -v gdb >/dev/null 2>&1; then
		echo "## gdb: thread apply all bt"
		gdb -p "${PID}" -batch -nx \
			-ex 'set pagination off' \
			-ex 'thread apply all bt 25' 2>&1 | grep -v '^\[New LWP' || true
	else
		echo "## gdb not installed: backtraces skipped (nix shell nixpkgs#gdb)"
	fi
} >"${OUT}"

echo "hang dump written: ${OUT}"
