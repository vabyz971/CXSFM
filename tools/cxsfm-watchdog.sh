#!/usr/bin/env bash
# CXSFM watchdog — inject libcxsfm.so into the RUNNING game.
#
# Why not LD_PRELOAD? Injecting after startup means the engine is fully
# initialized when our ctor fires: no loader-order races, no env
# inheritance by Steam helper processes, no guessing when the game is up.
#
# Flow: wait for the garage menu in Player.log, wait DELAY_SECS (click
# "TO CITY" meanwhile so HUD/scene objects exist), then ptrace-inject a
# fresh copy of the .so. Touch INJECT_NOW anytime to force a re-inject.
#
# Usage:
#   ./tools/cxsfm-watchdog.sh
#   CXSF_SO=/path/to/libcxsfm.so ./tools/cxsfm-watchdog.sh
set -euo pipefail

TOOLS_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(dirname "${TOOLS_DIR}")"
INJECT_BIN="${TOOLS_DIR}/cxsym-inject"
CXSF_SO="${CXSF_SO:-${REPO_ROOT}/target/release/libcxsfm.so}"
INJECT_NOW="${TOOLS_DIR}/INJECT_NOW"

# Garage / "TO CITY" screen (after world load).
GATE_LINE="${CXSF_GATE_LINE:-[GameStateManager]: MainMenuState}"

# Delay so injection lands in-game, not on the loading screen.
DELAY_SECS="${CXSF_DELAY_SECS:-15}"

GAME_PATH="${CXSF_GAME_PATH:-${HOME}/.config/unity3d/CarX Technologies/CarX Street}"
GAME_PATH="${GAME_PATH/#\~/${HOME}}"
PLAYER_LOG="${GAME_PATH}/Player.log"

# The injected .so copy MUST live somewhere the game's container can see.
# /tmp does NOT work: pressure-vessel gives the game a private /tmp, so
# dlopen fails with "No such file or directory" (proven in practice).
# $HOME is shared into the container (the CXSF_LOG file already proved it).
CACHE_DIR="${XDG_CACHE_HOME:-${HOME}/.cache}/cxsfm"
mkdir -p "${CACHE_DIR}"
# Stale copies from older sessions (successful injects keep theirs).
find "${CACHE_DIR}" -maxdepth 1 -name 'cxsfm-*.so' -mtime +1 -delete 2>/dev/null || true

if [[ ! -x "${INJECT_BIN}" ]]; then
	echo "injector not found. Run: make -C ${TOOLS_DIR}" >&2
	exit 1
fi
if [[ ! -f "${CXSF_SO}" ]]; then
	echo "libcxsfm.so not found at ${CXSF_SO}. Run: cargo build --release" >&2
	exit 1
fi

resolve_pid() {
	pgrep -n -f 'CarX_Street\.x86_64' || true
}

# Unique copy so the loader treats every inject as fresh. Lives in
# CACHE_DIR (container-visible), never /tmp (container-private).
do_inject() {
	local pid="$1"
	local so_copy
	so_copy="$(mktemp "${CACHE_DIR}/cxsfm-XXXXXX.so")"
	cp -f "${CXSF_SO}" "${so_copy}"
	chmod 755 "${so_copy}"
	echo "Injecting into game (pid ${pid})…"
	if "${INJECT_BIN}" "${pid}" "${so_copy}"; then
		echo "Inject OK. Set CXSF_LOG to keep the framework log."
		return 0
	fi
	echo "Inject failed." >&2
	rm -f "${so_copy}"
	return 1
}

echo "Game path: ${GAME_PATH}"
echo "Framework .so: ${CXSF_SO}"
echo "Delay after main menu: ${DELAY_SECS}s (override: CXSF_DELAY_SECS=N)"
echo "Start the game, reach the main menu, click TO CITY in time."
echo "Touch ${INJECT_NOW} anytime to force a re-inject."
echo "Waiting for game…"

last_auto_pid=""
log_pos=0
if [[ -f "${PLAYER_LOG}" ]]; then
	log_pos=$(wc -c <"${PLAYER_LOG}" | tr -d ' ')
fi

while true; do
	# Manual override — always allowed; clears the flag file.
	if [[ -f "${INJECT_NOW}" ]]; then
		pid="$(resolve_pid)"
		if [[ -n "${pid}" ]]; then
			echo "Manual inject requested."
			rm -f "${INJECT_NOW}"
			do_inject "${pid}" || true
			last_auto_pid="${pid}"
		else
			echo "Manual inject: game not running yet."
		fi
	fi

	# Reset when the game exits so the next session can auto-inject.
	if [[ -n "${last_auto_pid}" ]] && ! kill -0 "${last_auto_pid}" 2>/dev/null; then
		echo "Game closed; ready for next session."
		last_auto_pid=""
	fi

	if [[ -f "${PLAYER_LOG}" ]]; then
		size=$(wc -c <"${PLAYER_LOG}" | tr -d ' ')
		if (( size < log_pos )); then
			log_pos=0
		fi
		if (( size > log_pos )); then
			new="$(tail -c +"$((log_pos + 1))" "${PLAYER_LOG}" 2>/dev/null || true)"
			log_pos="${size}"
			if [[ -n "${new}" && "${new}" == *"${GATE_LINE}"* ]]; then
				pid="$(resolve_pid)"
				if [[ -n "${pid}" && "${pid}" != "${last_auto_pid}" ]]; then
					echo "Main menu seen — waiting ${DELAY_SECS}s…"
					sleep "${DELAY_SECS}"
					pid="$(resolve_pid)"
					if [[ -n "${pid}" && "${pid}" != "${last_auto_pid}" ]]; then
						if do_inject "${pid}"; then
							last_auto_pid="${pid}"
							echo "Auto-inject finished."
						fi
					else
						echo "Game gone or already handled after delay."
					fi
				fi
			fi
		fi
	fi

	sleep 0.5
done
