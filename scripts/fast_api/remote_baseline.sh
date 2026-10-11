#!/usr/bin/env bash
# Run INSIDE the parent-owned Arch container, never on the workstation.
set -euo pipefail
: "${HF_ROOT:?set HF_ROOT to a private directory under the mounted lab root}"
: "${HF_BASE_SOURCE:?set HF_BASE_SOURCE to the extracted pinned source}"
[[ $(. /etc/os-release; printf '%s' "$ID") == arch ]] || { echo 'requires Arch container' >&2; exit 1; }
[[ "$HF_ROOT" == /home/boxd/w11y-env/portal/hf ]] || { echo 'unexpected lab root' >&2; exit 1; }
mkdir -p "$HF_ROOT"/{home,cargo,rustup,tools,logs,artifacts,target-baseline,zig-cache,empty-config,empty-state,empty-runtime}
# Isolate all tool/provider paths from the copied seed's homes and configurations.
for key in $(compgen -e | sed -n '/^HERDR_/p;/^CLAUDE_/p;/^CODEX_/p;/^OPENAI_/p;/^ANTHROPIC_/p'); do unset "$key"; done
unset SSH_AUTH_SOCK SSH_AGENT_PID BASH_ENV ENV
export HOME="$HF_ROOT/home" CARGO_HOME="$HF_ROOT/cargo" RUSTUP_HOME="$HF_ROOT/rustup"
export XDG_CONFIG_HOME="$HF_ROOT/empty-config" XDG_STATE_HOME="$HF_ROOT/empty-state" XDG_RUNTIME_DIR="$HF_ROOT/empty-runtime"
export CARGO_BUILD_JOBS=8 CARGO_TARGET_DIR="$HF_ROOT/target-baseline"
export PATH="$CARGO_HOME/bin:$HF_ROOT/tools/zig:$HF_ROOT/tools/bun:/usr/bin:/bin"
export ZIG_GLOBAL_CACHE_DIR="$HF_ROOT/zig-cache" ZIG_LOCAL_CACHE_DIR="$HF_ROOT/zig-cache"
cd "$HF_BASE_SOURCE"
[[ $(sed -n 's/^channel = "\(.*\)"/\1/p' rust-toolchain.toml) == 1.96.1 ]]
[[ $(cat "$HF_ROOT/artifacts/source-base.txt") == 7b116c05bfda646af39d2524c54e70c751f57ee8 ]]
if [[ ! -x "$CARGO_HOME/bin/rustup" ]]; then
  curl --fail --location --proto '=https' --tlsv1.2 https://sh.rustup.rs -o "$HF_ROOT/tools/rustup-init.sh"
  sh "$HF_ROOT/tools/rustup-init.sh" -y --no-modify-path --profile minimal --default-toolchain 1.96.1 --component clippy --component rustfmt
fi
rustup toolchain install 1.96.1 --profile minimal --component clippy --component rustfmt
if [[ ! -x "$HF_ROOT/tools/zig/zig" ]]; then
  curl --fail --location https://ziglang.org/download/index.json -o "$HF_ROOT/tools/zig-index.json"
  python3 - "$HF_ROOT" <<'PY'
import hashlib,json,pathlib,subprocess,sys,tarfile,urllib.request
root=pathlib.Path(sys.argv[1]); data=json.loads((root/'tools/zig-index.json').read_text())['0.16.0']['x86_64-linux']
archive=root/'tools/zig.tar.xz'; urllib.request.urlretrieve(data['tarball'],archive)
assert hashlib.sha256(archive.read_bytes()).hexdigest()==data['shasum'], 'Zig checksum mismatch'
with tarfile.open(archive) as stream: stream.extractall(root/'tools/zig-extract',filter='data')
folder=next((root/'tools/zig-extract').iterdir()); folder.rename(root/'tools/zig')
(root/'artifacts/zig-download.json').write_text(json.dumps(data,indent=2))
PY
fi
export ZIG="$HF_ROOT/tools/zig/zig"
[[ $("$ZIG" version) == 0.16.0 ]]
# Install the recipe's runners privately. Cargo honors the same eight-job cap.
command -v just >/dev/null || cargo install just --locked
command -v cargo-nextest >/dev/null || cargo install cargo-nextest --locked
if [[ ! -x "$HF_ROOT/tools/bun/bun" ]]; then
  curl --fail --location https://github.com/oven-sh/bun/releases/download/bun-v1.3.14/bun-linux-x64.zip -o "$HF_ROOT/tools/bun.zip"
  python3 - "$HF_ROOT" <<'PY'
from pathlib import Path
import sys,zipfile
root=Path(sys.argv[1])
with zipfile.ZipFile(root/'tools/bun.zip') as stream:
    (root/'tools/bun').mkdir(exist_ok=True)
    (root/'tools/bun/bun').write_bytes(stream.read('bun-linux-x64/bun'))
(root/'tools/bun/bun').chmod(0o755)
PY
fi
{
  rustc --version --verbose
  cargo --version
  "$ZIG" version
  just --version
  cargo nextest --version
  bun --version
  cat /etc/os-release
  sha256sum Cargo.lock rust-toolchain.toml "$HF_ROOT/artifacts/published-herdr"
} > "$HF_ROOT/artifacts/toolchain.txt"
unset HERDR_BUILD_CHANNEL HERDR_BUILD_ID HERDR_BUILD_COMMIT
just build 2>&1 | tee "$HF_ROOT/logs/baseline-build.log"
cp "$CARGO_TARGET_DIR/release/herdr" "$HF_ROOT/artifacts/same-toolchain-baseline-herdr"
chmod 555 "$HF_ROOT/artifacts/same-toolchain-baseline-herdr" "$HF_ROOT/artifacts/published-herdr"
sha256sum "$HF_ROOT/artifacts/"*herdr > "$HF_ROOT/artifacts/binary-hashes.txt"
printf 'baseline build completed\n' > "$HF_ROOT/artifacts/baseline-complete.txt"
