#!/usr/bin/env python3
"""Sign OptChat RPM copies and assemble signed static DNF repositories."""
import argparse
import hashlib
import json
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tempfile
from urllib.parse import urlsplit


def capture(args, **kwargs):
    return subprocess.check_output(args, text=True, **kwargs).strip()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("packages", nargs="+", type=Path)
    parser.add_argument("--output", required=True, type=Path, help="New directory; existing output is never overwritten")
    parser.add_argument("--key", required=True, help="Full GPG signing-key fingerprint")
    parser.add_argument("--base-url", required=True, help="HTTPS URL at which this directory will be published")
    parser.add_argument("--allow-development", action="store_true")
    args = parser.parse_args()
    if not re.fullmatch(r"[0-9A-Fa-f]{40}|[0-9A-Fa-f]{64}", args.key):
        parser.error("provide a full GPG fingerprint")
    url = urlsplit(args.base_url)
    if url.scheme != "https" or not url.netloc or url.query or url.fragment or url.username or re.search(r"[\s\[\]$%]", args.base_url):
        parser.error("base URL must be a plain HTTPS repository URL without credentials, query or fragment")
    for tool in ["gpg", "rpm", "rpmkeys", "rpmsign", "createrepo_c"]:
        if not shutil.which(tool):
            parser.error(f"missing {tool}; use a signing workstation with RPM 4.19+ and createrepo_c")
    if "--key-id" not in capture(["rpmsign", "--help"]):
        parser.error("rpmsign must support --key-id (RPM 4.19+)")
    output = args.output.resolve()
    if output.exists():
        parser.error("use a new output directory")
    output.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="optchat-rpm-repo.", dir=output.parent) as temporary:
        temporary = Path(temporary)
        repo = temporary / "repository"
        repo.mkdir()
        public_key = subprocess.check_output(["gpg", "--armor", "--export", args.key])
        if not public_key:
            parser.error("key is not available to GPG")
        key_file = repo / "optchat-archive-keyring.asc"
        key_file.write_bytes(public_key)
        rpmdb = temporary / "verification-rpmdb"
        rpmdb.mkdir()
        subprocess.run(["rpm", "--dbpath", str(rpmdb), "--initdb"], check=True)
        subprocess.run(["rpm", "--dbpath", str(rpmdb), "--import", str(key_file)], check=True)
        manifest = []
        architectures = set()
        for source in args.packages:
            source = source.resolve()
            values = capture(["rpm", "-qp", "--qf", "%{NAME}\n%{VERSION}\n%{RELEASE}\n%{ARCH}\n%{PACKAGER}\n%{LICENSE}", str(source)]).splitlines()
            if len(values) != 6:
                parser.error("unexpected RPM metadata")
            name, version, revision, arch, maintainer, license_expression = values
            if name != "optchat" or arch not in ["x86_64", "aarch64"]:
                parser.error("repository accepts native optchat x86_64/aarch64 packages only")
            if not re.fullmatch(r"[0-9][A-Za-z0-9.+~]*", version) or not re.fullmatch(r"[A-Za-z0-9.+~]+", revision):
                parser.error("invalid package version/release")
            domain = maintainer.rsplit("@", 1)[-1].rstrip(">").lower()
            if not args.allow_development and ("0local" in revision or domain.endswith(".invalid") or domain in ["example.com", "example.org", "example.net"] or license_expression == "LicenseRef-Unspecified"):
                parser.error("development metadata cannot enter a production repository")
            package_dir = repo / arch / "packages"
            package_dir.mkdir(parents=True, exist_ok=True)
            package = package_dir / f"optchat-{version}-{revision}.{arch}.rpm"
            if package.exists():
                parser.error("duplicate package/version/architecture")
            shutil.copyfile(source, package)
            unsigned_digest = hashlib.sha256(package.read_bytes()).hexdigest()
            # Sign copies, leaving build outputs and their unsigned checksums unchanged.
            subprocess.run(["rpmsign", "--define", "_openpgp_sign gpg", "--key-id", args.key, "--resign", str(package)], check=True)
            verification = capture(["rpmkeys", "--dbpath", str(rpmdb), "--checksig", "--verbose", str(package)])
            if not re.search(r"(?im)signature.*:\s*OK", verification):
                parser.error("RPM signature verification did not confirm a signature")
            digest = hashlib.sha256(package.read_bytes()).hexdigest()
            package.with_name(package.name + ".sha256").write_text(f"{digest}  {package.name}\n")
            manifest.append({"package": str(package.relative_to(repo)), "sha256": digest, "unsigned_sha256": unsigned_digest,
                             "signing_key": args.key.upper(), "signature_verification": "passed"})
            architectures.add(arch)
        for arch in sorted(architectures):
            subprocess.run(["createrepo_c", str(repo / arch)], check=True)
            metadata = repo / arch / "repodata/repomd.xml"
            subprocess.run(["gpg", "--yes", "--armor", "--local-user", args.key, "--output", str(metadata) + ".asc", "--detach-sign", str(metadata)], check=True)
            subprocess.run(["gpg", "--verify", str(metadata) + ".asc", str(metadata)], check=True)
        (repo / "optchat.repo").write_text(
            "[optchat]\nname=OptChat\n"
            f"baseurl={args.base_url.rstrip('/')}/$basearch\n"
            "enabled=1\ngpgcheck=1\nrepo_gpgcheck=1\nsslverify=1\n"
            "gpgkey=file:///etc/pki/rpm-gpg/OPTCHAT-KEY\n"
        )
        (repo / "SIGNING-KEY.txt").write_text(args.key.upper() + "\n")
        (repo / "signed-packages.json").write_text(json.dumps(manifest, indent=2) + "\n")
        repo.rename(output)
    print(f"Prepared signed DNF repositories: {output}")
    print("Publish payloads/hashed metadata before repomd.xml and its signature; retain older files for active clients.")
    print("No repository was uploaded and no host RPM trust/database was modified.")


if __name__ == "__main__":
    try:
        main()
    except (OSError, subprocess.CalledProcessError) as error:
        sys.exit(str(error))
