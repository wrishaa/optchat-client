#!/usr/bin/env python3
"""Fetch the pinned packaging tools; never execute an unverified download."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time


def matches(path, asset):
    if not path.is_file() or path.stat().st_size != asset["size"]:
        return False
    with path.open("rb") as file:
        digest = hashlib.file_digest(file, "sha256").hexdigest()
    return digest == asset["sha256"]


def main():
    release_dir = Path(__file__).resolve().parent
    root = release_dir.parents[2]
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cache-dir", type=Path, default=root / "build/appimage-tools")
    args = parser.parse_args()
    args.cache_dir.mkdir(parents=True, exist_ok=True)
    assets = json.loads((release_dir / "appimage-tools.json").read_text())
    for asset in assets:
        if Path(asset["name"]).name != asset["name"]:
            raise RuntimeError("Invalid filename in packaging tool pins")
        destination = args.cache_dir / asset["name"]
        if matches(destination, asset):
            destination.chmod(0o755)
            print(f"Verified cached {asset['name']}", flush=True)
            continue
        descriptor, temporary = tempfile.mkstemp(dir=args.cache_dir, suffix=".download")
        os.close(descriptor)
        temporary = Path(temporary)
        try:
            print(f"Downloading {asset['name']}", flush=True)
            # The asset ID pins the release file; a fresh query avoids stale partial responses.
            url = asset["url"] + "?optchat_download=" + str(time.time_ns())
            subprocess.run([
                "curl", "--fail", "--location", "--silent", "--show-error",
                "--retry", "3", "--retry-delay", "1", "--connect-timeout", "15",
                "--max-time", "300", "--header", "Accept: application/octet-stream",
                "--header", "Cache-Control: no-cache", url, "--output", str(temporary),
            ], check=True)
            if not matches(temporary, asset):
                raise RuntimeError(f"Incomplete or changed download: {asset['name']}; expected size/SHA256 did not match")
            temporary.chmod(0o755)
            temporary.replace(destination)
            print(f"Verified downloaded {asset['name']}", flush=True)
        finally:
            temporary.unlink(missing_ok=True)


if __name__ == "__main__":
    try:
        main()
    except (OSError, RuntimeError, subprocess.CalledProcessError) as error:
        sys.exit(str(error))
