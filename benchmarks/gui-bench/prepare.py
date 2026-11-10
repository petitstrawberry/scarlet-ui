#!/usr/bin/env python3
"""Prepare a pinned gui-bench checkout without modifying an existing checkout."""
import argparse
import json
from pathlib import Path
import shutil
import subprocess

UPSTREAM = "https://github.com/zortax/gui-bench"
REVISION = "221ba14287ac99110a13cccf5b8aa38e2881fff7"
HERE = Path(__file__).resolve().parent
SCARLET_UI = HERE.parents[1]


def command(*args, cwd=None):
    subprocess.run(args, cwd=cwd, check=True)


def prepare(destination, source=UPSTREAM, all_frameworks=False):
    destination = destination.resolve()
    # Never reset, clean, patch or overwrite a user's existing working tree.
    destination.mkdir(parents=True, exist_ok=False)
    try:
        command("git", "init", "--quiet", str(destination))
        command("git", "remote", "add", "origin", source, cwd=destination)
        command("git", "fetch", "--depth", "1", "origin", REVISION, cwd=destination)
        command("git", "checkout", "--quiet", "--detach", "FETCH_HEAD", cwd=destination)
        command("git", "apply", "--check", str(HERE / "upstream.patch"), cwd=destination)
        command("git", "apply", str(HERE / "upstream.patch"), cwd=destination)

        adapter = destination / "adapters" / "scarletui"
        shutil.copytree(HERE / "src", adapter / "src")
        manifest = (HERE / "Cargo.toml.in").read_text().replace(
            "@SCARLET_UI_PATH@", json.dumps(str(SCARLET_UI / "crates" / "scarlet-ui"))
        )
        (adapter / "Cargo.toml").write_text(manifest)
        shutil.copy2(HERE / "README.md", adapter / "README.md")

        manifest_path = destination / "Cargo.toml"
        manifest = manifest_path.read_text()
        # Keep optimized code, thin LTO and one codegen unit, without large
        # debug-symbol archives when compiling three independent GUI stacks.
        manifest = manifest.replace("debug = 1", "debug = 0")
        if not all_frameworks:
            # Cargo resolves every workspace member even with `-p`. Avoid
            # requiring zgui's external checkout or GPUI's graph for a smoke run.
            manifest = manifest.replace('    "adapters/zgui",\n', "")
            manifest = manifest.replace('    "adapters/gpui",\n', "")
            manifest = manifest.replace('exclude = ["vendor/gpui-ce"]',
                'exclude = ["vendor/gpui-ce", "adapters/zgui", "adapters/gpui"]')
        manifest = manifest.replace('    "crates/gui-bench",\n',
            '    "crates/gui-bench",\n    "adapters/scarletui",\n', 1)
        manifest_path.write_text(manifest)
        config = destination / ".cargo" / "config.toml"
        with config.open("a") as stream:
            stream.write('\n[env]\nSCARLET_UI_BENCH_REPO = ' + json.dumps(str(SCARLET_UI)) + '\n')
    except BaseException:
        # This function created the entire directory; there are no user edits.
        shutil.rmtree(destination)
        raise
    return destination


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("destination", type=Path, help="new directory for the prepared checkout")
    parser.add_argument("--source", default=UPSTREAM, help="upstream URL or local mirror (revision is always pinned)")
    parser.add_argument("--all-frameworks", action="store_true", help="retain zgui/GPUI members; requires ../zgui as upstream documents")
    args = parser.parse_args()
    destination = prepare(args.destination, args.source, args.all_frameworks)
    print(f"Prepared {destination} at {REVISION}")
    print("Run from that directory (without a shared CARGO_TARGET_DIR):")
    print("  cargo run -p gui-bench -- run --framework scarletui --preset smoke --repetitions 1")


if __name__ == "__main__":
    main()
