#!/usr/bin/env python3
"""Derive package versions from Cargo and validated platform tags."""
import argparse
import json
import os
from pathlib import Path
import re
import tomllib

ROOT = Path(__file__).resolve().parents[1]


def plan(platform, ref, run_number, version):
    if not re.fullmatch(r"\d+\.\d+\.\d+", version) or run_number < 1:
        raise ValueError("expected a numeric Cargo version and positive run number")
    tag = ref.removeprefix("refs/tags/") if ref.startswith("refs/tags/") else ""
    result = {"version": version, "tag": tag, "publish": bool(tag), "production": False}
    if platform == "linux":
        match = re.fullmatch(rf"linux-v{re.escape(version)}-preview\.([1-9]\d*)", tag)
        if tag and not match:
            raise ValueError(f"use linux-v{version}-preview.<positive revision>; stable Linux release metadata is separate")
        current = int(match[1]) if match else run_number * 2 + 1
        result.update(previous_revision=f"0local.{current - 1}", revision=f"0local.{current}")
    else:
        match = re.fullmatch(rf"android-v{re.escape(version)}(?:(-preview)\.|\.)([1-9]\d*)", tag)
        if tag and not match:
            raise ValueError(f"use android-v{version}-preview.<code> or android-v{version}.<code>")
        code = int(match[2]) if match else run_number + 1
        if code > 2_100_000_000:
            raise ValueError("Android versionCode exceeds 2100000000")
        production = bool(match and not match[1])
        name = f"{version}.{code}" if production else f"{version}-dev.{code}"
        result.update(version_code=code, version_name=name, production=production)
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("platform", choices=["linux", "android"])
    parser.add_argument("--ref", default=os.environ.get("GITHUB_REF", "refs/heads/local"))
    parser.add_argument("--run-number", type=int, default=int(os.environ.get("GITHUB_RUN_NUMBER", "1")))
    parser.add_argument("--github-output", type=Path)
    args = parser.parse_args()
    version = tomllib.loads((ROOT / "Cargo.toml").read_text())["workspace"]["package"]["version"]
    try:
        result = plan(args.platform, args.ref, args.run_number, version)
    except ValueError as error:
        parser.error(str(error))
    if args.github_output:
        with args.github_output.open("a") as file:
            for key, value in result.items():
                file.write(f"{key}={str(value).lower() if isinstance(value, bool) else value}\n")
    print(json.dumps(result, indent=2))


if __name__ == "__main__":
    main()
