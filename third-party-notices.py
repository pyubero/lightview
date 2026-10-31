#!/usr/bin/env python3
"""Prints the licences every lightview package carries: its own, the fonts' and those of every Rust
crate built for the given target (Linux by default).

Identical crate texts are printed once, under the crates that use them. Fails if a crate has no text.
"""
import json
import re
import subprocess
import sys
from pathlib import Path

# Windows defaults to cp1252, which can't hold these texts.
sys.stdout.reconfigure(encoding="utf-8", newline="\n")
ROOT = Path(__file__).parent
TARGET = sys.argv[1] if len(sys.argv) > 1 else "x86_64-unknown-linux-gnu"
NAME = re.compile(r"(licen[cs]e|copying|notice)", re.I)
# Published without their licence file; the same project's or author's other crate has the same text.
SIBLING = {"alloc-stdlib": "alloc-no-stdlib", "clipboard-win": "error-code"}


def license_files(crate_dir):
    for entry in sorted(crate_dir.iterdir()):
        if NAME.match(entry.name):
            yield from sorted(f for f in entry.rglob("*") if f.is_file()) if entry.is_dir() else [entry]


meta = json.loads(subprocess.check_output(
    ["cargo", "metadata", "--locked", "--format-version", "1", "--filter-platform", TARGET], cwd=ROOT))
used = {node["id"] for node in meta["resolve"]["nodes"]}
packages = sorted((p for p in meta["packages"] if p["id"] in used and p["name"] != "lightview"), key=lambda p: (p["name"], p["version"]))
dirs = {p["name"]: Path(p["manifest_path"]).parent for p in packages}

texts = {}  # text -> crates using it
missing = []
for p in packages:
    label = f'{p["name"]} {p["version"]} ({p["license"]})'
    crate_dir = dirs[SIBLING[p["name"]]] if p["name"] in SIBLING else Path(p["manifest_path"]).parent
    files = list(license_files(crate_dir))
    for f in files:
        texts.setdefault(f.read_text(encoding="utf-8", errors="replace").strip(), []).append(label)
    if not files:
        missing.append((p, label))

apache = next(t for t in texts if "TERMS AND CONDITIONS FOR USE, REPRODUCTION" in t)
for p, label in missing:
    # These crates are all offered under Apache-2.0, which needs no copyright line of its own.
    if "Apache-2.0" not in p["license"]:
        sys.exit(f"no licence text for {label}")
    texts[apache].append(label + ", used under Apache-2.0")

listed = {label.split(" (")[0] for labels in texts.values() for label in labels}
assert listed == {f'{p["name"]} {p["version"]}' for p in packages}

print("lightview\nCopyright (C) 2026 Pablo Yubero\nLicensed under the GNU General Public License, version 3 or later.\n")
for name in ["LICENSE", "fonts/NOTICE.txt", "fonts/LICENSE-APACHE-2.0.txt", "fonts/emoji-icon-font-mit-license.txt"]:
    sys.stdout.write((ROOT / name).read_text(encoding="utf-8"))
    if name in ("LICENSE", "fonts/emoji-icon-font-mit-license.txt"):
        print("\n")
print(f"lightview uses these {len(packages)} third-party Rust crates. Their licences follow.\n")
for text, labels in sorted(texts.items(), key=lambda kv: kv[1][0]):
    print("=" * 78)
    print("\n".join(sorted(set(labels))))
    print("-" * 78)
    print(text)
    print()
