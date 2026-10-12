#!/usr/bin/env python3
"""Launch only the packaged downstream binary in a private Linux namespace."""
import hashlib
import json
import os
from pathlib import Path
import re
import sys

SESSION = "fast"
# Explicit directory selectors in src/integration/env.rs. Keep the source-backed
# contract in test_packaging.py aligned when Herdr adds a provider selector.
PROVIDER_HOME_OVERRIDES = frozenset({
    "PI_CODING_AGENT_DIR", "PI_CONFIG_DIR", "CLAUDE_CONFIG_DIR", "CODEX_HOME",
    "KIMI_CODE_HOME", "COPILOT_HOME", "QODER_CONFIG_DIR", "QWEN_HOME",
    "CURSOR_CONFIG_DIR", "ANTIGRAVITY_CLI_CONFIG_DIR", "GROK_CONFIG_DIR",
    "GROK_HOME", "HERMES_HOME",
})
# These are also home/config fallbacks in integration/env.rs and config/io.rs.
# HOME and XDG homes are replaced below; Windows fallbacks must not survive.
HOME_FALLBACK_OVERRIDES = frozenset({
    "APPDATA", "LOCALAPPDATA", "USERPROFILE", "HOMEDRIVE", "HOMEPATH", "PROGRAMDATA",
})


def reject_routing(args):
    # Herdr parses routing flags anywhere before --, including after a command.
    options = args[:args.index("--")] if "--" in args else args
    for arg in options:
        if arg.split("=", 1)[0] in {"--session", "--remote", "--remote-keybindings",
                                     "--handoff-import", "--import-exe", "--machine"}:
            raise ValueError(f"routing override is unavailable in this package: {arg}")
    if options and options[0] in {"update", "channel", "session", "machine",
                                  "remote-api-bridge", "remote-client-bridge"}:
        raise ValueError("use a reviewed downstream package for updates; routing stays private")
    if options[:2] == ["server", "update-agent-manifests"]:
        raise ValueError("this package keeps its reviewed bundled detection manifests")


def private_environment(package, namespace, inherited):
    if not re.fullmatch(r"[a-z0-9][a-z0-9-]{0,23}", namespace):
        raise ValueError("namespace must be 1-24 lowercase letters, digits or hyphens")
    # A child Herdr CLI already inherits this home. Keep it in the same silo.
    inherited_home = Path(inherited["HOME"]).absolute()
    namespace_root = inherited_home.parent if (inherited_home / ".fast-api-home").is_file() else (
        inherited_home / ".local/share/herdr-fast-api" / namespace)
    # Reject pre-existing symlink components before creating or reading anything.
    for path in [namespace_root, *namespace_root.parents]:
        if path.is_symlink():
            raise ValueError(f"namespace must not traverse symlinks: {path}")
    home = namespace_root / "h"
    config = namespace_root / "c"
    data = config / "herdr" / "sessions" / SESSION
    if len(os.fsencode(data / "herdr-handoff-2147483647.sock")) > 107:
        raise ValueError("private socket path exceeds Linux's Unix socket path limit")
    for path in (namespace_root, home, config, data, namespace_root / "s",
                 namespace_root / "r", namespace_root / "d", namespace_root / "k"):
        for component in [path, *path.parents]:
            if component.is_symlink():
                raise ValueError(f"namespace must not traverse symlinks: {component}")
        path.mkdir(mode=0o700, parents=True, exist_ok=True)
        if path.stat().st_uid != os.getuid():
            raise ValueError(f"namespace is not owned by this user: {path}")
        path.chmod(0o700)
    for path in (home / ".fast-api-home", data / "herdr.sock", data / "herdr-client.sock",
                 data / "session.json"):
        if path.is_symlink():
            raise ValueError(f"namespace file must not be a symlink: {path}")
    (home / ".fast-api-home").touch(mode=0o600)
    config_file = config / "config.toml"
    if config_file.is_symlink():
        raise ValueError("private configuration must not be a symlink")
    if not config_file.exists():
        with config_file.open("x") as stream:
            stream.write("onboarding = false\n[update]\nversion_check = false\nmanifest_check = false\n")
    # No inherited pane identity, remote target, API overrides or provider homes.
    env = {key: value for key, value in inherited.items()
           if not key.startswith(("HERDR_", "CLAUDE_", "CODEX_", "OPENAI_", "ANTHROPIC_"))
           and key not in PROVIDER_HOME_OVERRIDES | HOME_FALLBACK_OVERRIDES
           and key not in {"SSH_AUTH_SOCK", "SSH_AGENT_PID", "BASH_ENV", "ENV", "ZDOTDIR"}}
    env.update(HOME=str(home), XDG_CONFIG_HOME=str(config),
               XDG_STATE_HOME=str(namespace_root / "s"),
               XDG_RUNTIME_DIR=str(namespace_root / "r"),
               XDG_DATA_HOME=str(namespace_root / "d"),
               XDG_CACHE_HOME=str(namespace_root / "k"),
               HERDR_CONFIG_PATH=str(config_file), HERDR_SESSION=SESSION,
               PATH=str(package / "bin") + ":" + inherited.get("PATH", "/usr/bin:/bin"))
    return env, data


def main():
    args = sys.argv[1:]
    namespace = "main"
    print_route = False
    while args:
        if args[0] == "--namespace":
            if len(args) < 2:
                raise ValueError("--namespace requires a value")
            namespace, args = args[1], args[2:]
        elif args[0] == "--print-route":
            print_route, args = True, args[1:]
        else:
            break
    if args[:1] == ["--"]:
        args = args[1:]
    reject_routing(args)
    package = Path(__file__).resolve().parents[1]
    metadata = json.loads((package / "manifest.json").read_text())
    binary = package / "libexec/herdr"
    actual = hashlib.sha256(binary.read_bytes()).hexdigest()
    if actual != metadata["binary_sha256"]:
        raise ValueError("packaged executable differs from its recorded hash")
    env, data = private_environment(package, namespace, dict(os.environ))
    if print_route:
        print(json.dumps({"binary": str(binary), "manifest": metadata,
                          "api_socket": str(data / "herdr.sock"),
                          "client_socket": str(data / "herdr-client.sock"),
                          "session_dir": str(data), "environment": {
                              k: env[k] for k in ("HOME", "HERDR_CONFIG_PATH", "HERDR_SESSION",
                                                   "XDG_CONFIG_HOME", "XDG_STATE_HOME",
                                                   "XDG_RUNTIME_DIR", "XDG_DATA_HOME")}}, indent=2))
        return
    os.execve(binary, [str(binary), "--session", SESSION, *args], env)


if __name__ == "__main__":
    try:
        main()
    except (ValueError, OSError, KeyError) as error:
        sys.exit(f"herdr-fast-api: {error}")
