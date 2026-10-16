#!/usr/bin/env python3
"""Assemble a signed static APT repository from tested OptChat Debian packages."""
import argparse
from datetime import datetime, timezone
from email.utils import format_datetime
import gzip
import hashlib
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tempfile


def capture(args, **kwargs):
    return subprocess.check_output(args, text=True, **kwargs).strip()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("packages", nargs="+", type=Path)
    parser.add_argument("--output", required=True, type=Path, help="New output directory to upload over HTTPS")
    parser.add_argument("--key", required=True, help="Your GPG signing-key fingerprint")
    parser.add_argument("--suite", default="stable")
    parser.add_argument("--allow-development", action="store_true", help="Permit 0local or example.invalid metadata in a test repository")
    args = parser.parse_args()
    if not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9._-]*", args.suite):
        parser.error("invalid suite")
    if not re.fullmatch(r"[0-9A-Fa-f]{40}|[0-9A-Fa-f]{64}", args.key):
        parser.error("provide a full GPG fingerprint")
    for executable in ["gpg", "dpkg-deb", "dpkg-scanpackages"]:
        if not shutil.which(executable):
            parser.error(f"missing {executable}")
    output = args.output.resolve()
    if output.exists():
        parser.error("use a new output directory; existing repositories are not overwritten")
    output.parent.mkdir(parents=True, exist_ok=True)
    # A temporary sibling means incomplete or unsigned metadata is never published as the output.
    with tempfile.TemporaryDirectory(prefix="optchat-apt.", dir=output.parent) as temporary:
        temporary = Path(temporary)
        repo = temporary / "repository"
        pool = repo / "pool/main/o/optchat"
        pool.mkdir(parents=True)
        architectures = set()
        for package in args.packages:
            package = package.resolve()
            name = capture(["dpkg-deb", "--field", str(package), "Package"])
            version = capture(["dpkg-deb", "--field", str(package), "Version"])
            maintainer = capture(["dpkg-deb", "--field", str(package), "Maintainer"])
            arch = capture(["dpkg-deb", "--field", str(package), "Architecture"])
            if not re.fullmatch(r"[0-9][A-Za-z0-9.+:~\-]*", version):
                parser.error("invalid package version")
            if name != "optchat" or arch not in ["amd64", "arm64"]:
                parser.error("repository accepts native optchat amd64/arm64 packages only")
            domain = maintainer.rsplit("@", 1)[-1].rstrip(">").lower()
            if not args.allow_development and ("0local" in version or domain.endswith(".invalid") or domain in ["example.com", "example.org", "example.net"]):
                parser.error("development package cannot enter a production repository")
            destination = pool / f"optchat_{version}_{arch}.deb"
            if destination.exists():
                parser.error("duplicate package/version/architecture")
            shutil.copyfile(package, destination)
            architectures.add(arch)
        dist = repo / "dists" / args.suite
        index_paths = []
        for arch in sorted(architectures):
            index_dir = dist / "main" / ("binary-" + arch)
            index_dir.mkdir(parents=True)
            data = subprocess.check_output(["dpkg-scanpackages", "--arch", arch, "pool", "/dev/null"], cwd=repo)
            (index_dir / "Packages").write_bytes(data)
            (index_dir / "Packages.gz").write_bytes(gzip.compress(data, mtime=0))
            index_paths.extend([index_dir / "Packages", index_dir / "Packages.gz"])
        release = ["Origin: OptChat", "Label: OptChat", "Suite: " + args.suite,
                   "Codename: " + args.suite, "Architectures: " + " ".join(sorted(architectures)),
                   "Components: main", "Date: " + format_datetime(datetime.now(timezone.utc), usegmt=True),
                   "Description: OptChat native Linux packages", "Acquire-By-Hash: yes", "SHA256:"]
        for path in index_paths:
            data = path.read_bytes()
            digest = hashlib.sha256(data).hexdigest()
            release.append(f" {digest} {len(data)} {path.relative_to(dist).as_posix()}")
            by_hash = path.parent / "by-hash/SHA256" / digest
            by_hash.parent.mkdir(parents=True, exist_ok=True)
            by_hash.write_bytes(data)
        (dist / "Release").write_text("\n".join(release) + "\n")
        subprocess.run(["gpg", "--yes", "--local-user", args.key, "--output", str(dist / "InRelease"), "--clearsign", str(dist / "Release")], check=True)
        subprocess.run(["gpg", "--yes", "--armor", "--local-user", args.key, "--output", str(dist / "Release.gpg"), "--detach-sign", str(dist / "Release")], check=True)
        public_key = subprocess.check_output(["gpg", "--armor", "--export", args.key])
        if not public_key:
            parser.error("signing key could not be exported")
        (repo / "optchat-archive-keyring.asc").write_bytes(public_key)
        (repo / "SIGNING-KEY.txt").write_text(args.key.upper() + "\n")
        # All release metadata is signed before the prepared directory becomes visible.
        repo.rename(output)
    print(f"Prepared signed repository: {output}")
    print("Upload its contents to your HTTPS repository address after release qualification.")
    print("Keep older by-hash indices/package files when deploying updates; switch signed metadata last.")
    print("Signing fingerprint:", args.key.upper())


if __name__ == "__main__":
    try:
        main()
    except (OSError, subprocess.CalledProcessError) as error:
        sys.exit(str(error))
