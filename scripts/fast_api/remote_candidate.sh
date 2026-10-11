#!/usr/bin/env bash
# Parent submits this only inside the assigned Arch container after baseline setup.
set -euo pipefail
R=/home/boxd/w11y-env/portal
HF_ROOT="$R/hf"
[[ $(. /etc/os-release; printf '%s' "$ID") == arch ]] || { echo 'requires Arch container' >&2; exit 1; }
[[ -f "$HF_ROOT/artifacts/baseline-complete.txt" ]]
for key in $(compgen -e | sed -n '/^HERDR_/p;/^CLAUDE_/p;/^CODEX_/p;/^OPENAI_/p;/^ANTHROPIC_/p'); do unset "$key"; done
unset SSH_AUTH_SOCK SSH_AGENT_PID BASH_ENV ENV
export HOME="$HF_ROOT/candidate-home" CARGO_HOME="$HF_ROOT/cargo" RUSTUP_HOME="$HF_ROOT/rustup"
export XDG_CONFIG_HOME="$HOME/c" XDG_STATE_HOME="$HOME/s" XDG_RUNTIME_DIR="$HOME/r" XDG_DATA_HOME="$HOME/d" XDG_CACHE_HOME="$HOME/k"
mkdir -p "$HOME" "$XDG_CONFIG_HOME" "$XDG_STATE_HOME" "$XDG_RUNTIME_DIR" "$XDG_DATA_HOME" "$XDG_CACHE_HOME"
export PATH="$CARGO_HOME/bin:$HF_ROOT/tools/zig:$HF_ROOT/tools/bun:/usr/bin:/bin"
export ZIG="$HF_ROOT/tools/zig/zig" CARGO_BUILD_JOBS=8 CARGO_TARGET_DIR="$HF_ROOT/target-candidate"
export ZIG_GLOBAL_CACHE_DIR="$HF_ROOT/zig-cache" ZIG_LOCAL_CACHE_DIR="$HF_ROOT/zig-cache"
export HERDR_BUILD_CHANNEL=fast-api HERDR_BUILD_COMMIT="$(cat "$HF_ROOT/artifacts/candidate-source.txt")"
export HERDR_BUILD_ID="${HERDR_BUILD_COMMIT:0:12}"
case "${1:-}" in
  check)
    just check
    python3 -m unittest scripts.fast_api.test_packaging
    ;;
  linux-check)
    # Parent must record that this deliberately omits Windows cross-lint.
    just ci
    just docs-contract-test
    python3 -m unittest scripts.fast_api.test_packaging
    ;;
  build)
    just build
    sha256sum "$CARGO_TARGET_DIR/release/herdr" > "$HF_ROOT/artifacts/candidate-binary-hash.txt"
    ;;
  package)
    python3 scripts/fast_api/package.py --binary "$CARGO_TARGET_DIR/release/herdr" \
      --source-commit "$HERDR_BUILD_COMMIT" --toolchain-record "$HF_ROOT/artifacts/toolchain.txt" \
      --output "$HF_ROOT/artifacts/herdr-fast-api-$HERDR_BUILD_ID"
    ;;
  *) echo 'usage: remote_candidate.sh check|linux-check|build|package' >&2; exit 2 ;;
esac
