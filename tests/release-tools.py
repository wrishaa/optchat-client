#!/usr/bin/env python3
"""Release version, provenance and download-guide tests; fixtures are not installable packages."""
import hashlib
import importlib.util
import json
from pathlib import Path
import re
import sys
import tempfile
import tomllib
import unittest

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "tools"))
from release_version import plan
from prepare_linux_release import prepare as prepare_release

spec = importlib.util.spec_from_file_location("download", ROOT / "shells/linux/release/prepare_download.py")
download = importlib.util.module_from_spec(spec)
spec.loader.exec_module(download)
VERSION = tomllib.loads((ROOT / "Cargo.toml").read_text())["workspace"]["package"]["version"]
COMMIT = "a" * 40


class Versions(unittest.TestCase):
    def test_preview_and_production_tags(self):
        linux = plan("linux", f"refs/tags/linux-v{VERSION}-preview.8", 99, VERSION)
        self.assertEqual((linux["previous_revision"], linux["revision"]), ("0local.7", "0local.8"))
        preview = plan("android", f"refs/tags/android-v{VERSION}-preview.8", 99, VERSION)
        production = plan("android", f"refs/tags/android-v{VERSION}.8", 99, VERSION)
        self.assertEqual(preview["version_code"], production["version_code"])
        self.assertFalse(preview["production"])
        self.assertTrue(production["production"])
        self.assertEqual(production["version_name"], VERSION + ".8")

    def test_branch_runs_increase_versions(self):
        for platform, field in [("linux", "revision"), ("android", "version_code")]:
            values = [plan(platform, "refs/heads/local", n, VERSION)[field] for n in [1, 2, 9, 10]]
            values = [int(v.split(".")[-1]) if isinstance(v, str) else v for v in values]
            self.assertEqual(values, sorted(set(values)))

    def test_invalid_tags_and_codes(self):
        for platform, tag in [("linux", f"linux-v{VERSION}-preview.0"), ("linux", "linux-v99.0.0-preview.8"),
                              ("android", f"android-v{VERSION}.2100000001"), ("android", "android-v99.0.0.8"),
                              ("android", f"android-v{VERSION}.8\nmalformed")]:
            with self.subTest(tag=tag), self.assertRaises(ValueError):
                plan(platform, "refs/tags/" + tag, 1, VERSION)


class ReleaseFiles(unittest.TestCase):
    def setUp(self):
        self.scratch = tempfile.TemporaryDirectory()
        self.addCleanup(self.scratch.cleanup)
        self.root = Path(self.scratch.name)
        self.downloads = self.root / "downloads"
        for kind, runner, arch in [("deb", "x86_64", "amd64"), ("deb", "aarch64", "arm64"),
                                  ("rpm", "x86_64", "x86_64"), ("rpm", "aarch64", "aarch64")]:
            folder = self.downloads / f"linux-{kind}-{runner}"
            (folder / "build-info").mkdir(parents=True)
            name = f"optchat_{VERSION}-0local.7_{arch}.deb" if kind == "deb" else f"optchat-{VERSION}-0local.7.fc43.{arch}.rpm"
            package = folder / name
            package.write_bytes(b"Synthetic release fixture; not an installer")
            digest = hashlib.sha256(package.read_bytes()).hexdigest()
            Path(str(package) + ".sha256").write_text(f"{digest}  {name}\n")
            report = {"format": kind, "architecture": arch, "version": f"{VERSION}-0local.7" + (".fc43" if kind == "rpm" else ""),
                      "sha256": digest, "source_revision": COMMIT, "source_has_local_changes": False}
            (folder / "build-info" / (name + ".build.json")).write_text(json.dumps(report))
            (folder / "INSTALL.md").write_text("Fixture instructions\n")
        self.output = self.root / "release"

    def release(self):
        prepare_release(self.downloads, self.output, f"linux-v{VERSION}-preview.7", COMMIT, "https://example.invalid/ci")

    def report(self):
        return next(self.downloads.rglob("*.build.json"))

    def test_complete_matrix(self):
        self.release()
        manifest = json.loads((self.output / "linux-release.json").read_text())
        self.assertEqual(len(manifest["packages"]), 4)
        self.assertEqual(len(list(self.output.glob("*-INSTALL.md"))), 4)

    def test_source_and_version_mismatches_rejected(self):
        file = self.report()
        original = json.loads(file.read_text())
        for key, value in [("source_revision", "b" * 40), ("source_has_local_changes", True), ("version", "99.0.0")]:
            with self.subTest(key=key):
                file.write_text(json.dumps(original | {key: value}))
                with self.assertRaises(ValueError): self.release()
                self.assertFalse(self.output.exists())
        file.write_text(json.dumps(original))

    def test_tampered_package_rejected(self):
        next(self.downloads.rglob("*.deb")).write_bytes(b"tampered")
        with self.assertRaises(ValueError): self.release()
        self.assertFalse(self.output.exists())

    def test_missing_architecture_rejected(self):
        next((self.downloads / "linux-deb-aarch64").glob("*.deb")).unlink()
        with self.assertRaises(ValueError): self.release()
        self.assertFalse(self.output.exists())

    def test_existing_release_not_overwritten(self):
        self.release()
        with self.assertRaises(ValueError): self.release()

    def test_embedded_guide_has_no_missing_relative_links(self):
        package = next(self.downloads.rglob("*.deb"))
        report = package.parent / "build-info" / (package.name + ".build.json")
        Path(str(package) + ".build.json").write_bytes(report.read_bytes())
        output = self.root / "bundle"
        download.prepare(package, output)
        guide = (output / "INSTALL.md").read_text()
        for target in re.findall(r"\]\(([^)]+)\)", guide):
            self.assertTrue("://" in target or target.startswith(("#", "mailto:")), target)
        self.assertIn("/blob/", guide)
        self.assertEqual(len(re.findall(r"^# ", guide, re.M)), 1)


if __name__ == "__main__":
    unittest.main()
