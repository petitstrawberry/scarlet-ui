"""Exercise toolchain acquisition without Nix, network access, or real sleeps."""
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

SCRIPT = Path(__file__).with_name("install-scarlet-toolchain.sh")


class InstallScarletToolchainTests(unittest.TestCase):
    def run_install(self, failures):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            (root / "nix").write_text("""#!/usr/bin/env bash
set -eu
count=0
if [[ -f "$TEST_ROOT/count" ]]; then count=$(cat "$TEST_ROOT/count"); fi
count=$((count + 1))
printf '%s' "$count" > "$TEST_ROOT/count"
printf '%s\\n' "$*" >> "$TEST_ROOT/args"
if (( count <= FAILURES )); then
    echo 'partial output must not reach GITHUB_PATH'
    echo 'error: unable to download cached toolchain: HTTP error 416' >&2
    exit 42
fi
printf '/nix/store/pinned-toolchain\\n'
""")
            (root / "sleep").write_text("""#!/usr/bin/env bash
printf '%s\\n' "$*" >> "$TEST_ROOT/sleeps"
""")
            for name in ("nix", "sleep"):
                (root / name).chmod(0o755)
            path_file = root / "github-path"
            path_file.write_text("/existing/bin\n")
            result = subprocess.run(
                ["bash", str(SCRIPT)],
                env={
                    **os.environ,
                    "PATH": f"{root}{os.pathsep}{os.environ['PATH']}",
                    "TEST_ROOT": str(root),
                    "FAILURES": str(failures),
                    "SCARLET_TOOLCHAIN": "github:owner/repo/pinned-revision#toolchain",
                    "GITHUB_PATH": str(path_file),
                },
                capture_output=True,
                text=True,
            )
            attempts = int((root / "count").read_text())
            args = (root / "args").read_text().splitlines()
            sleeps = (root / "sleeps").read_text().splitlines() if (root / "sleeps").exists() else []
            for invocation in args:
                self.assertEqual(
                    invocation,
                    "build github:owner/repo/pinned-revision#toolchain^out --no-link --print-out-paths",
                )
            return result, attempts, path_file.read_text(), sleeps

    def test_success_does_not_retry(self):
        result, attempts, paths, sleeps = self.run_install(0)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(attempts, 1)
        self.assertEqual(paths, "/existing/bin\n/nix/store/pinned-toolchain/bin\n")
        self.assertEqual(sleeps, [])

    def test_transient_failure_retries_without_publishing_partial_output(self):
        for failures in (1, 2):
            with self.subTest(failures=failures):
                result, attempts, paths, sleeps = self.run_install(failures)
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(attempts, failures + 1)
                self.assertEqual(paths, "/existing/bin\n/nix/store/pinned-toolchain/bin\n")
                self.assertEqual(sleeps, ["10", "20"][:failures])

    def test_persistent_failure_stays_failed_and_does_not_change_path(self):
        result, attempts, paths, sleeps = self.run_install(3)
        self.assertEqual(result.returncode, 42)
        self.assertEqual(attempts, 3)
        self.assertEqual(paths, "/existing/bin\n")
        self.assertEqual(sleeps, ["10", "20"])
        self.assertIn("failed after 3 attempts", result.stderr)


if __name__ == "__main__":
    unittest.main()
