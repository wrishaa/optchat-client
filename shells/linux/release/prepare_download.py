#!/usr/bin/env python3
"""Create a self-contained Linux installer download with verified metadata and installation instructions."""
import argparse
import hashlib
import json
from pathlib import Path
import re
import shlex
import shutil
import sys
import subprocess
from urllib.parse import quote, urlsplit

ROOT = Path(__file__).resolve().parents[3]
ARCHITECTURES = {
    "deb": {"amd64": "64-bit Intel/AMD (uname -m: x86_64)", "arm64": "64-bit ARM (uname -m: aarch64)"},
    "rpm": {"x86_64": "64-bit Intel/AMD (uname -m: x86_64)", "aarch64": "64-bit ARM (uname -m: aarch64)"},
}


def prepare(package, output):
    package = package.resolve()
    checksum = package.with_name(package.name + ".sha256")
    metadata = package.with_name(package.name + ".build.json")
    if not re.fullmatch(r"optchat[A-Za-z0-9_.+~-]+\.(deb|rpm)", package.name):
        raise ValueError("expected an OptChat DEB or RPM filename")
    report = json.loads(metadata.read_text())
    package_format = package.suffix[1:]
    architecture = report["architecture"]
    if report["format"] != package_format or architecture not in ARCHITECTURES[package_format]:
        raise ValueError("package format/architecture does not match its build report")
    separator = "_" if package_format == "deb" else "."
    if not package.name.endswith(f"{separator}{architecture}.{package_format}"):
        raise ValueError("package filename does not match its reported architecture")
    with package.open("rb") as stream:
        digest = hashlib.file_digest(stream, "sha256").hexdigest()
    if report["sha256"] != digest or checksum.read_text().strip() != f"{digest}  {package.name}":
        raise ValueError("package, checksum and build report do not agree")
    if output.exists() and any(output.iterdir()):
        raise ValueError("output directory must be new or empty")
    output.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(package, output / package.name)
    shutil.copyfile(checksum, output / checksum.name)
    info = output / "build-info"
    info.mkdir()
    shutil.copyfile(metadata, info / metadata.name)
    for name in ["base-image.json", "builder-image.json"]:
        source = package.parent / name
        if source.is_file():
            shutil.copyfile(source, info / name)
    system = "Ubuntu 24.04/26.04 or Debian 13" if package_format == "deb" else "Fedora 43/44"
    install = "sudo apt update\n" if package_format == "deb" else ""
    install += f"sudo {'apt' if package_format == 'deb' else 'dnf'} install {shlex.quote('./' + package.name)}"
    guide = (ROOT / "shells/linux/README.md").read_text()
    # The guide is embedded outside the source tree; point repository links at the
    # exact documentation commit rather than leaving links to absent local files.
    commit = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip()
    def repository_link(match):
        label, target = match.groups()
        parsed = urlsplit(target)
        if parsed.scheme or parsed.netloc or target.startswith("#"):
            return match[0]
        path = (ROOT / "shells/linux" / parsed.path).resolve().relative_to(ROOT)
        url = f"https://github.com/fabihatasneem/optchat-client/blob/{commit}/{quote(path.as_posix())}"
        if parsed.fragment:
            url += "#" + parsed.fragment
        return f"[{label}]({url})"
    guide = re.sub(r"\[([^\]]+)\]\(([^)]+)\)", repository_link, guide)
    guide = "\n".join("#" + line if line.startswith("#") else line for line in guide.splitlines()) + "\n"
    (output / "INSTALL.md").write_text(
        "# Install this OptChat download\n\n"
        f"System: **{system}**. Processor: **{ARCHITECTURES[package_format][architecture]}**.\n\n"
        f"Installer: `{package.name}`. Open a terminal in this extracted folder.\n\n"
        "Check the download first:\n\n```bash\n"
        f"sha256sum -c {shlex.quote(checksum.name)}\n```\n\n"
        "Continue only if the result says **OK**. Then install:\n\n```bash\n"
        f"{install}\n```\n\n"
        "Open **OptChat** from your application menu, or run `optchat` as your regular user.\n"
        "Ask your engine administrator for a client token, then choose **Workspaces → Add engine** and paste it.\n"
        "Your desktop password wallet/keyring must be running and unlocked to save the token.\n\n"
        "No Rust, Deno, Podman or source checkout is needed to install this package.\n\n"
        "---\n\n" + guide
    )
    print(f"User download ready: {output.resolve()}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("package", type=Path)
    parser.add_argument("--output-dir", type=Path, required=True)
    args = parser.parse_args()
    try:
        prepare(args.package, args.output_dir)
    except (OSError, ValueError, KeyError) as error:
        parser.exit(1, f"Cannot prepare download: {error}\n")


if __name__ == "__main__":
    main()
