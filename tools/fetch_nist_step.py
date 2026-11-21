#!/usr/bin/env python3
# SPDX-License-Identifier: LGPL-2.1-or-later
"""Download the NIST MBE PMI STEP test models for the kernel bake-off.

NIST publishes the CAD models and STEP files of its MBE PMI Validation and
Conformance Testing project (CTC 1-5, FTC 6-11, STC 6-10) for use "without
any restrictions" (https://www.nist.gov/disclaimer; please acknowledge NIST
when using them). The same package NIST links from its MBE PMI page ships in
NIST's own STEP File Analyzer repository (https://github.com/usnistgov/SFA,
Release/NIST-PMI-STEP-Files.zip), which keeps its history, so a commit pins
the exact files. Two releases are pinned:

  current  SFA commit 1236017 (2026-11-02; package updated 2026-02-18): 28 STEP files
  2026-01  SFA commit c218911 (2026-01-12): 33 STEP files, byte-identical to
           NIST's 2024 package (5 AP203 files with graphic PMI and the earlier
           CTC 1, CTC 4, CTC 5, FTC 8 tessellated, FTC 11 and STC 9 files that
           the February 2026 update replaced or removed)

Usage:
  python3 tools/fetch_nist_step.py DEST            # both releases
  python3 tools/fetch_nist_step.py DEST --release current
  python3 tools/fetch_nist_step.py DEST --zip current=PATH  # a copy already on disk

DEST/<release>/ receives the zip's STEP files, flattened (the AP203 files of a
release keep their names, which do not clash with the AP242 ones). Files that
the 2026-01 release shares byte-for-byte with the current one are not
repeated in DEST/2026-01/ unless --all is given, so each file is tested once.
Every download is checked against the SHA-256 pinned below; a mismatch stops
the script with nothing extracted.

Reads untrusted archives: member names are reduced to their base name, only
*.stp members are written, and sizes are capped.
"""

import argparse
import hashlib
import io
import pathlib
import sys
import urllib.request
import zipfile

RELEASES = {
    "current": (
        "1236017676c0fa1f24f0fb4642fb088437a406f7",
        "1fb91bb8ff0fe02032b948fda0775bc74591cd0bebc0988347d32574e5884f90",
    ),
    "2026-01": (
        "c2189113abd35765e7a646ead516b6e6e8eaec65",
        "8fa78429e6d8d9b0d7681d223b6aa9ec98c3772185c55b1a0e3679b21c181911",
    ),
}
URL = "https://raw.githubusercontent.com/usnistgov/SFA/{commit}/Release/NIST-PMI-STEP-Files.zip"
MAX_ZIP = 64 * 1024 * 1024
MAX_MEMBER = 32 * 1024 * 1024


def fetch(release: str, local: str | None) -> bytes:
    commit, digest = RELEASES[release]
    if local:
        data = pathlib.Path(local).read_bytes()
    else:
        url = URL.format(commit=commit)
        print(f"downloading {url}", file=sys.stderr)
        with urllib.request.urlopen(url, timeout=120) as r:  # noqa: S310 (pinned https URL)
            data = r.read(MAX_ZIP + 1)
    if len(data) > MAX_ZIP:
        sys.exit(f"{release}: archive larger than {MAX_ZIP} bytes")
    got = hashlib.sha256(data).hexdigest()
    if got != digest:
        sys.exit(f"{release}: SHA-256 {got} does not match the pinned {digest}")
    return data


def step_members(data: bytes) -> dict[str, bytes]:
    out = {}
    with zipfile.ZipFile(io.BytesIO(data)) as z:
        for info in z.infolist():
            name = pathlib.PurePosixPath(info.filename).name
            if not name.lower().endswith(".stp") or info.file_size > MAX_MEMBER:
                continue
            if name in out:
                sys.exit(f"two members named {name}")
            out[name] = z.read(info)
    return out


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("dest", type=pathlib.Path)
    ap.add_argument("--release", choices=[*RELEASES, "all"], default="all")
    ap.add_argument("--zip", action="append", default=[], metavar="RELEASE=PATH",
                    help="use a local copy of a release's zip (still checksum-verified)")
    ap.add_argument("--all", action="store_true",
                    help="also write 2026-01 files identical to current ones")
    args = ap.parse_args()
    local = dict(z.split("=", 1) for z in args.zip)
    wanted = list(RELEASES) if args.release == "all" else [args.release]
    current: dict[str, bytes] = {}
    if "2026-01" in wanted and not args.all:
        current = step_members(fetch("current", local.get("current")))
    for release in wanted:
        members = current if release == "current" and current else step_members(fetch(release, local.get(release)))
        if release == "current":
            current = members
        seen = set(current.values()) if release == "2026-01" and not args.all else set()
        out = args.dest / release
        out.mkdir(parents=True, exist_ok=True)
        written = 0
        for name, data in sorted(members.items()):
            if data in seen:
                continue
            (out / name).write_bytes(data)
            written += 1
        print(f"{release}: {written} STEP files in {out}", file=sys.stderr)


if __name__ == "__main__":
    main()
