#!/usr/bin/env bash
# Runs every check on one checkout and prints one line per check. Usage: check.sh [CHECKOUT]
set -u
cd "${1:-$(git -C "$(dirname "$(readlink -f "$0")")" rev-parse --show-toplevel)}" || exit 2
export CARGO_BUILD_JOBS=8
# WebKit leaves one sandbox folder per web process in XDG_RUNTIME_DIR; keep them out of the user's.
# The path must stay short: WebKit binds a socket under it and socket paths stop at 108 characters.
mkdir -p "$HOME/.cache/crt" && XDG_RUNTIME_DIR=$(mktemp -d -p "$HOME/.cache/crt" XXXXXX) && chmod 700 "$XDG_RUNTIME_DIR" && export XDG_RUNTIME_DIR
trap 'rm -rf "$XDG_RUNTIME_DIR"' EXIT
head=$(git rev-parse --short HEAD); branch=$(git branch --show-current)
echo "== $branch @ $head"
status=0
run() { local label=$1; shift; if out=$("$@" 2>&1); then echo "ok    $label"; else echo "FAIL  $label"; echo "$out" | tail -15; status=1; fi; }
run "fmt" cargo fmt --check
run "clippy (all targets, gui-smoke)" cargo clippy --locked --all-targets --features gui-smoke -- -D warnings
if out=$(cargo test --locked 2>&1); then echo "ok    unit tests: $(echo "$out" | grep -E '^test result' | awk '{s+=$4} END {print s}') passed"; else echo "FAIL  unit tests"; echo "$out" | tail -20; status=1; fi
for suite in $(grep -oP '(?<=^name = ")[a-z_]+_smoke' Cargo.toml); do
  if out=$(env -u WAYLAND_DISPLAY GDK_BACKEND=x11 xvfb-run -n 200 -a -s '-screen 0 1440x1000x24' cargo test --locked --features gui-smoke --test "$suite" 2>&1); then
    echo "ok    $suite: $(echo "$out" | grep -c '^PASS') PASS lines"
  else echo "FAIL  $suite"; echo "$out" | grep -vE 'MESA|DRI3' | tail -15; status=1; fi
done
[ -z "$(git status --short)" ] && echo "ok    worktree clean" || { echo "FAIL  worktree dirty"; status=1; }
exit $status
