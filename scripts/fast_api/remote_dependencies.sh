#!/usr/bin/env bash
# Run as root in a disposable container of the exact existing Arch image, binding
# only hf. Installs tools, never compiles or runs Herdr/tests/benchmarks.
set -euo pipefail
HF_ROOT=/home/boxd/w11y-env/portal/hf
[[ $(. /etc/os-release; printf '%s' "$ID") == arch ]]
[[ $(id -u) == 0 ]]
[[ -f "$HF_ROOT/artifacts/baseline-complete.txt" ]]
[[ ! -e "$HF_ROOT/tools/node" && ! -e "$HF_ROOT/tools/ssh" ]]
unset SSH_AUTH_SOCK SSH_AGENT_PID BASH_ENV ENV
export HOME="$HF_ROOT/dependency-home"
export XDG_CONFIG_HOME="$HOME/c" XDG_STATE_HOME="$HOME/s" XDG_DATA_HOME="$HOME/d"
mkdir -p "$HOME" "$HF_ROOT/dependency-downloads/arch" "$HF_ROOT/logs/dependencies"
exec > >(tee "$HF_ROOT/logs/dependencies/setup.log") 2>&1
NODE_VERSION=22.13.1
NODE_ARCHIVE="node-v$NODE_VERSION-linux-x64.tar.xz"
NODE_URL="https://nodejs.org/dist/v$NODE_VERSION"
curl --fail --location "$NODE_URL/SHASUMS256.txt" -o "$HF_ROOT/dependency-downloads/node-SHASUMS256.txt"
curl --fail --location "$NODE_URL/$NODE_ARCHIVE" -o "$HF_ROOT/dependency-downloads/$NODE_ARCHIVE"
awk -v archive="$NODE_ARCHIVE" '$2 == archive { print }' \
  "$HF_ROOT/dependency-downloads/node-SHASUMS256.txt" > "$HF_ROOT/dependency-downloads/node-checksum.txt"
[[ $(wc -l < "$HF_ROOT/dependency-downloads/node-checksum.txt") == 1 ]]
(cd "$HF_ROOT/dependency-downloads"; sha256sum -c node-checksum.txt)
mkdir "$HF_ROOT/tools/node"
tar -xJf "$HF_ROOT/dependency-downloads/$NODE_ARCHIVE" \
  --strip-components=1 -C "$HF_ROOT/tools/node"

# Only this disposable Arch container gets package installation. Preserve the
# signed package downloads and dependency closure; never change the helper image.
pacman -Sy --needed --noconfirm --cachedir "$HF_ROOT/dependency-downloads/arch" openssh
pacman -Q > "$HF_ROOT/logs/dependencies/arch-packages.txt"
sha256sum "$HF_ROOT/dependency-downloads/arch/"*.pkg.tar.zst \
  > "$HF_ROOT/logs/dependencies/arch-download-hashes.txt"
mkdir -p "$HF_ROOT/tools/ssh/bin" "$HF_ROOT/tools/ssh/libexec" "$HF_ROOT/tools/ssh/lib"
cp /usr/bin/ssh "$HF_ROOT/tools/ssh/libexec/ssh"
ldd /usr/bin/ssh | tee "$HF_ROOT/logs/dependencies/ssh-libraries.txt"
! grep -q 'not found' "$HF_ROOT/logs/dependencies/ssh-libraries.txt"
# Bundle the loader and all linked libraries from the same installed environment.
# A private loader invocation keeps these libraries away from cargo/Node/Herdr.
awk '/=> \/|^[[:space:]]*\// { for (i=1; i<=NF; i++) if ($i ~ /^\//) print $i }' \
  "$HF_ROOT/logs/dependencies/ssh-libraries.txt" |
  while IFS= read -r library; do cp -L "$library" "$HF_ROOT/tools/ssh/lib/"; done
[[ -f "$HF_ROOT/tools/ssh/lib/ld-linux-x86-64.so.2" ]]
cat > "$HF_ROOT/tools/ssh/bin/ssh" <<'SH'
#!/usr/bin/env bash
set -euo pipefail
root=/home/boxd/w11y-env/portal/hf/tools/ssh
exec "$root/lib/ld-linux-x86-64.so.2" --library-path "$root/lib" "$root/libexec/ssh" "$@"
SH
chmod 755 "$HF_ROOT/tools/ssh/bin/ssh"
export PATH="$HF_ROOT/tools/node/bin:$HF_ROOT/tools/ssh/bin:/usr/bin:/bin"
node --version
ssh -V
find "$HF_ROOT/tools/node" "$HF_ROOT/tools/ssh" -type f -exec sha256sum '{}' + \
  > "$HF_ROOT/logs/dependencies/tool-file-hashes.txt"
printf 'Node %s; signed Arch OpenSSH; dependency setup only\n' "$NODE_VERSION" \
  > "$HF_ROOT/artifacts/dependencies-complete.txt"
