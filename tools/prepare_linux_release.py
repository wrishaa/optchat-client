#!/usr/bin/env python3
"""Verify the four native CI downloads and assemble a Linux preview release."""
import argparse
import hashlib
import json
from pathlib import Path
import re
import shutil
from release_version import ROOT, plan
import tomllib


def prepare(downloads, output, tag, source_commit, run_url):
    version = tomllib.loads((ROOT / "Cargo.toml").read_text())["workspace"]["package"]["version"]
    release = plan("linux", "refs/tags/" + tag, 1, version)
    if not re.fullmatch(r"[0-9a-f]{40}", source_commit):
        raise ValueError("expected the exact source commit")
    if output.exists() and any(output.iterdir()):
        raise ValueError("output must be new or empty")
    packages = []
    expected = [("deb", "x86_64", "amd64"), ("deb", "aarch64", "arm64"),
                ("rpm", "x86_64", "x86_64"), ("rpm", "aarch64", "aarch64")]
    for kind, runner_arch, package_arch in expected:
        directory = downloads / f"linux-{kind}-{runner_arch}"
        candidates = list(directory.glob(f"*.{kind}"))
        if len(candidates) != 1:
            raise ValueError(f"expected one {kind} installer in {directory}")
        package = candidates[0]
        name = (f"optchat_{version}-{release['revision']}_{package_arch}.deb" if kind == "deb" else
                f"optchat-{version}-{release['revision']}.fc43.{package_arch}.rpm")
        if package.name != name:
            raise ValueError(f"package does not match tag: {package.name}")
        report = json.loads((directory / "build-info" / (name + ".build.json")).read_text())
        digest = hashlib.sha256(package.read_bytes()).hexdigest()
        checksum = directory / (name + ".sha256")
        expected_version = f"{version}-{release['revision']}" + (".fc43" if kind == "rpm" else "")
        if (report["format"] != kind or report["architecture"] != package_arch or
                report["version"] != expected_version or not (directory / "INSTALL.md").is_file() or
                report["source_revision"] != source_commit or report["source_has_local_changes"] is not False or
                report["sha256"] != digest or checksum.read_text().strip() != f"{digest}  {name}"):
            raise ValueError(f"source, architecture or checksum mismatch: {name}")
        packages.append({"file": name, "sha256": digest, "size_bytes": package.stat().st_size,
                         "ci_artifact": directory.name, "build_report": report})
    output.mkdir(parents=True, exist_ok=True)
    for entry in packages:
        directory = downloads / entry["ci_artifact"]
        name = entry["file"]
        for file in [directory / name, directory / (name + ".sha256"), directory / "build-info" / (name + ".build.json")]:
            shutil.copyfile(file, output / file.name)
        shutil.copyfile(directory / "INSTALL.md", output / (entry["ci_artifact"] + "-INSTALL.md"))
    manifest = {"application": "OptChat", "version": version, "package_revision": release["revision"],
                "release_tag": tag, "channel": "development-preview", "source_revision": source_commit,
                "ci_run_url": run_url, "validation": {"native_builds": "passed", "container_package_lifecycle": "passed",
                "fedora_tls": "passed", "manual_desktop": "pending", "real_engine": "pending", "production_signing": "pending"},
                "packages": packages}
    (output / "linux-release.json").write_text(json.dumps(manifest, indent=2) + "\n")
    (output / "release-notes.md").write_text(
        f"OptChat {version}, Linux preview {release['revision']}.\n\n"
        "Choose DEB for Ubuntu/Debian or RPM for Fedora, then amd64/x86_64 for Intel/AMD or arm64/aarch64 for ARM.\n"
        "Checksums, installation guides and build provenance accompany the four installers.\n\n"
        "Unsigned development preview. Native package lifecycle and Fedora TLS checks passed; desktop and real-engine sign-off remain pending.\n"
        "Review the qualification record before approving distribution. This automation creates a draft only.\n")


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("downloads", type=Path)
    p.add_argument("--output-dir", type=Path, required=True)
    p.add_argument("--tag", required=True)
    p.add_argument("--source-commit", required=True)
    p.add_argument("--ci-run-url", required=True)
    args = p.parse_args()
    try:
        prepare(args.downloads, args.output_dir, args.tag, args.source_commit, args.ci_run_url)
    except (OSError, ValueError, KeyError) as error:
        p.error(str(error))


if __name__ == "__main__":
    main()
