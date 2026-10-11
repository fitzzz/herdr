#!/usr/bin/env python3
"""Stage a versioned package; never build, install globally or replace a package."""
import argparse
import hashlib
import json
from pathlib import Path
import shutil
import subprocess

BASE = "7b116c05bfda646af39d2524c54e70c751f57ee8"


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def package(binary, output, source_commit, toolchain_record):
    binary = binary.resolve(strict=True)
    # Remote caller supplies a recorded release artifact, never an installed path.
    if output.exists():
        raise ValueError("output already exists; choose a new versioned package path")
    version = subprocess.check_output([str(binary), "--version"], text=True).strip()
    if "fast-api" not in version:
        raise ValueError("candidate needs HERDR_BUILD_CHANNEL=fast-api build identity")
    output.mkdir(parents=True, mode=0o755)
    (output / "bin").mkdir()
    (output / "libexec").mkdir()
    shutil.copyfile(binary, output / "libexec/herdr")
    shutil.copyfile(Path(__file__).with_name("launcher.py"), output / "bin/herdr-fast-api")
    (output / "bin/herdr").symlink_to("herdr-fast-api")
    manifest = {"schema": 1, "upstream_base": BASE, "source_commit": source_commit,
                "version": version, "binary_sha256": sha(output / "libexec/herdr"),
                "launcher_sha256": sha(output / "bin/herdr-fast-api"),
                "toolchain": toolchain_record.read_text(), "app_dir": "herdr"}
    (output / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
    # Atomic upstream replacement requires write permission on this directory.
    for path in (output / "libexec/herdr", output / "bin/herdr-fast-api"):
        path.chmod(0o555)
    (output / "manifest.json").chmod(0o444)
    for path in (output / "libexec", output / "bin", output):
        path.chmod(0o555)
    return manifest


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--source-commit", required=True)
    parser.add_argument("--toolchain-record", type=Path, required=True)
    opts = parser.parse_args()
    print(json.dumps(package(opts.binary, opts.output, opts.source_commit,
                             opts.toolchain_record), indent=2))
