"""Run remotely: python3 -m unittest scripts.fast_api.test_packaging"""
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

from scripts.fast_api.launcher import private_environment, reject_routing
from scripts.fast_api.package import package


class NamespaceTests(unittest.TestCase):
    def setUp(self):
        self.base = Path(tempfile.mkdtemp(prefix="hf-"))
        self.package = self.base / "pkg"
        self.inherited = {"HOME": str(self.base / "u"), "PATH": "/usr/bin:/bin",
                          "HERDR_SOCKET_PATH": "/live/api.sock",
                          "HERDR_CLIENT_SOCKET_PATH": "/live/client.sock",
                          "HERDR_SESSION": "live", "HERDR_PANE_ID": "w1:p1",
                          "HERDR_CONFIG_PATH": "/live/config.toml",
                          "CODEX_HOME": "/live/codex", "CLAUDE_CONFIG_DIR": "/live/claude",
                          "XDG_CONFIG_HOME": "/live/config", "XDG_STATE_HOME": "/live/state"}

    def tearDown(self):
        shutil.rmtree(self.base)

    def test_inherited_live_routes_and_provider_homes_cannot_escape(self):
        env, data = private_environment(self.package, "main", self.inherited)
        for key in ("HERDR_SOCKET_PATH", "HERDR_CLIENT_SOCKET_PATH", "HERDR_PANE_ID",
                    "CODEX_HOME", "CLAUDE_CONFIG_DIR"):
            self.assertNotIn(key, env)
        for key in ("HOME", "XDG_CONFIG_HOME", "XDG_STATE_HOME", "XDG_RUNTIME_DIR",
                    "XDG_DATA_HOME", "HERDR_CONFIG_PATH"):
            self.assertTrue(Path(env[key]).is_relative_to(self.base / "u"))
        self.assertEqual(env["HERDR_SESSION"], "fast")
        self.assertTrue(str(data).endswith("/c/herdr/sessions/fast"))
        self.assertEqual(os.stat(data).st_mode & 0o777, 0o700)
        self.assertEqual(env["PATH"].split(":")[0], str(self.package / "bin"))

    def test_child_cli_keeps_the_same_namespace(self):
        first, first_data = private_environment(self.package, "scratch", self.inherited)
        second, second_data = private_environment(self.package, "main", first)
        self.assertEqual(first_data, second_data)
        self.assertEqual(first["HOME"], second["HOME"])

    def test_symlink_component_cannot_route_to_existing_state(self):
        target = self.base / "live"
        target.mkdir()
        local = self.base / "u/.local/share"
        local.mkdir(parents=True)
        (local / "herdr-fast-api").symlink_to(target, target_is_directory=True)
        with self.assertRaisesRegex(ValueError, "symlink"):
            private_environment(self.package, "main", self.inherited)
        self.assertEqual(list(target.iterdir()), [])

    def test_cli_routing_and_upstream_update_are_refused_before_launch(self):
        for args in (["update", "--handoff"], ["channel", "set", "preview"],
                     ["status", "--session=default"], ["--remote", "host"],
                     ["--machine=live", "pane", "list"], ["session", "attach", "default"],
                     ["server", "--handoff-import", "/live", "token"]):
            with self.subTest(args=args), self.assertRaises(ValueError):
                reject_routing(args)
        # Text following -- belongs to the pane command and retains input order.
        reject_routing(["pane", "send", "w1:p1", "--", "--remote", "update"])


class PackageTests(unittest.TestCase):
    def setUp(self):
        self.base = Path(tempfile.mkdtemp(prefix="hfp-"))
        self.binary = self.base / "source-binary"
        self.binary.write_text("#!/bin/sh\necho herdr 0.9.3-fast-api.test\n")
        self.binary.chmod(0o755)
        self.toolchain = self.base / "toolchain.txt"
        self.toolchain.write_text("fixture toolchain\n")
        self.output = self.base / "pkg"

    def tearDown(self):
        # Production never relaxes package permissions; only this test owns them.
        for path in (self.output, self.output / "bin", self.output / "libexec"):
            if path.exists():
                path.chmod(0o755)
        shutil.rmtree(self.base)

    def test_package_preserves_provenance_and_refuses_overwrite(self):
        manifest = package(self.binary, self.output, "a" * 40, self.toolchain)
        self.assertEqual(manifest["source_commit"], "a" * 40)
        self.assertEqual(manifest, json.loads((self.output / "manifest.json").read_text()))
        self.assertEqual((self.output / "bin/herdr").resolve(), self.output / "bin/herdr-fast-api")
        with self.assertRaisesRegex(ValueError, "already exists"):
            package(self.binary, self.output, "b" * 40, self.toolchain)

    @unittest.skipIf(os.getuid() == 0, "permission contract requires the delivery's unprivileged user")
    def test_raw_binary_updater_cannot_stage_or_atomically_replace_itself(self):
        package(self.binary, self.output, "a" * 40, self.toolchain)
        replacement = self.base / "upstream"
        replacement.write_text("upstream binary")
        with self.assertRaises(PermissionError):
            (self.output / "libexec/herdr-update.tmp").write_bytes(b"upstream")
        with self.assertRaises(PermissionError):
            os.replace(replacement, self.output / "libexec/herdr")

    def test_launcher_does_not_execute_tampered_artifact(self):
        package(self.binary, self.output, "a" * 40, self.toolchain)
        binary = self.output / "libexec/herdr"
        binary.chmod(0o755)
        binary.write_text("#!/bin/sh\necho SHOULD_NOT_EXECUTE\n")
        result = subprocess.run([str(self.output / "bin/herdr-fast-api"), "--version"],
                                capture_output=True, text=True)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("differs from", result.stderr)
        self.assertNotIn("SHOULD_NOT_EXECUTE", result.stdout)


if __name__ == "__main__":
    unittest.main()
