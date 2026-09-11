#!/usr/bin/env bash
#
# Cross-builds the Windows binary from WSL, stages it on the Windows drive and launches it
# there. Running the exe from the Windows filesystem rather than over \\wsl.localhost keeps
# start-up fast and lets Windows lock the file without blocking the next build.
#
# Usage: scripts/win-run.sh [-n] [--debug] [--wait] [audio-file]
#
#   -n, --no-run   build and stage only, do not launch
#       --debug    debug build: console subsystem, so log output is visible
#       --wait     run in the foreground and relay the app's output to this terminal
#
# The staging directory defaults to %USERPROFILE%\Switchblade; override with SWITCHBLADE_WIN_DIR
# (a WSL path). RUST_LOG is forwarded to the app and defaults to warn.

set -euo pipefail

TARGET=x86_64-pc-windows-gnu
PROFILE=release
run=1
wait_for_exit=0
file=

while [ $# -gt 0 ]; do
    case "$1" in
        -n|--no-run) run=0 ;;
        --debug) PROFILE=debug ;;
        --wait) wait_for_exit=1 ;;
        -h|--help) awk 'NR>1 && /^#/ {sub(/^# ?/, ""); print; next} NR>1 {exit}' "$0"; exit 0 ;;
        --) shift; file=${1:-}; break ;;
        -*) echo "unknown option: $1" >&2; exit 2 ;;
        *) file=$1 ;;
    esac
    shift
done

die() { echo "win-run: $*" >&2; exit 1; }

command -v cmd.exe >/dev/null 2>&1 || die "needs WSL with Windows interop enabled (cmd.exe not found)"
rustup target list --installed | grep -qx "$TARGET" \
    || die "missing target: run 'rustup target add $TARGET'"
command -v x86_64-w64-mingw32-gcc >/dev/null 2>&1 \
    || die "missing linker: install mingw-w64 (apt install mingw-w64)"

cd "$(dirname "$0")/.."

if [ "$PROFILE" = release ]; then
    cargo build --release --target "$TARGET"
else
    cargo build --target "$TARGET"
fi
exe=target/$TARGET/$PROFILE/switchblade.exe
[ -f "$exe" ] || die "build produced no $exe"

if [ -n "${SWITCHBLADE_WIN_DIR:-}" ]; then
    dest_dir=$SWITCHBLADE_WIN_DIR
else
    profile_win=$(cmd.exe /c 'echo %USERPROFILE%' 2>/dev/null | tr -d '\r\n')
    [ -n "$profile_win" ] || die "could not read %USERPROFILE% from Windows"
    dest_dir=$(wslpath -u "$profile_win")/Switchblade
fi
mkdir -p "$dest_dir"
dest=$dest_dir/switchblade.exe

# Windows locks a running exe against overwrite but still allows a rename, so move any
# previous copy aside first: a staged build then succeeds even with the app still open.
if [ -e "$dest" ]; then
    mv -f "$dest" "$dest.old" 2>/dev/null || die "$dest is in use; close Switchblade and retry"
fi
cp "$exe" "$dest"
rm -f "$dest.old" 2>/dev/null || true

dest_win=$(wslpath -w "$dest")
echo "staged $(du -h "$dest" | cut -f1) -> $dest_win"

[ "$run" -eq 1 ] || exit 0

file_win=
if [ -n "$file" ]; then
    [ -e "$file" ] || die "no such file: $file"
    file_win=$(wslpath -w "$(realpath "$file")")
fi

# Windows tools reject a WSL working directory, so every launch runs from the staging directory.
cd "$dest_dir"

if [ "$wait_for_exit" -eq 1 ]; then
    # Interop relays the app's output back here, and holds this shell until the window closes.
    echo "running (close the window to return)..."
    WSLENV=${WSLENV:+$WSLENV:}RUST_LOG RUST_LOG=${RUST_LOG:-warn} "$dest" ${file_win:+"$file_win"}
    exit $?
fi

# `cmd.exe /c start` is denied when invoked through interop, so detach via PowerShell instead.
if command -v powershell.exe >/dev/null 2>&1; then
    ps_quote() { printf "'%s'" "$(printf '%s' "$1" | sed "s/'/''/g")"; }
    launch="\$env:RUST_LOG=$(ps_quote "${RUST_LOG:-warn}"); Start-Process -FilePath $(ps_quote "$dest_win")"
    [ -n "$file_win" ] && launch="$launch -ArgumentList $(ps_quote "$file_win")"
    powershell.exe -NoProfile -Command "$launch" >/dev/null
else
    # No PowerShell: run through interop in its own session so it outlives this shell.
    WSLENV=${WSLENV:+$WSLENV:}RUST_LOG RUST_LOG=${RUST_LOG:-warn} \
        setsid "$dest" ${file_win:+"$file_win"} >/dev/null 2>&1 &
fi
echo "launched on Windows"
