"""Opt-in remote proof with an already-built candidate, using synthetic homes only.

HERDR_FAST_API_TEST_BINARY=/absolute/candidate/herdr python3 -m unittest \
    scripts.fast_api.test_launcher_integration
"""
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

from scripts.fast_api.launcher import private_environment
from scripts.fast_api.package import package
from scripts.fast_api.test_packaging import PROVIDER_OVERRIDE_FIXTURE, HOME_FALLBACK_FIXTURE


def snapshot(root):
    return {str(path.relative_to(root)): path.read_bytes() if path.is_file() else None
            for path in root.rglob("*")}


class IntegrationIsolationTests(unittest.TestCase):
    def test_pi_install_under_private_launcher_cannot_write_external_provider_homes(self):
        # Missing explicit artifact is an error, never a silently skipped proof.
        binary = Path(os.environ["HERDR_FAST_API_TEST_BINARY"]).resolve(strict=True)
        base = Path(tempfile.mkdtemp(prefix="hfi-"))
        output = base / "pkg"
        try:
            external = base / "external"
            external.mkdir()
            inherited = dict(os.environ, HOME=str(base / "u"))
            for key in PROVIDER_OVERRIDE_FIXTURE + HOME_FALLBACK_FIXTURE:
                directory = external / key
                directory.mkdir()
                (directory / "sentinel").write_bytes(b"synthetic original provider state\n")
                inherited[key] = str(directory)
            # Pi's override points at a valid writable synthetic installation.
            # Without scrubbing, the CLI succeeds by writing here instead.
            (external / "PI_CODING_AGENT_DIR/extensions").mkdir()
            original = snapshot(external)
            toolchain = base / "toolchain.txt"
            toolchain.write_text("isolated integration fixture; prebuilt artifact only\n")
            package(binary, output, "test-fixture", toolchain)
            env, _ = private_environment(output, "main", inherited)
            private_pi = Path(env["HOME"]) / ".pi/agent/extensions"
            private_pi.mkdir(parents=True)
            result = subprocess.run([str(output / "bin/herdr-fast-api"),
                                     "integration", "install", "pi"],
                                    cwd=base, env=inherited, text=True, capture_output=True)
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            self.assertEqual(snapshot(external), original)
            installed = private_pi / "herdr-agent-state.ts"
            self.assertTrue(installed.is_file(), result.stdout + result.stderr)
            self.assertIn("HERDR_INTEGRATION_ID=pi", installed.read_text())
        finally:
            for path in (output, output / "bin", output / "libexec"):
                if path.exists():
                    path.chmod(0o755)
            shutil.rmtree(base)


if __name__ == "__main__":
    unittest.main()
