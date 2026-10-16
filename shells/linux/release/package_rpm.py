#!/usr/bin/env python3
"""Build a Fedora-native OptChat RPM with automatically generated ELF dependencies."""
import argparse
from datetime import datetime, timezone
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tarfile
import tempfile
from package_common import ROOT, build_binary, command, copy_manual, copy_notices, default_maintainer, is_development, publish, validate_metadata


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--maintainer", default=default_maintainer(), help="Package maintainer; overrides OPTCHAT_PACKAGE_MAINTAINER and the approved default")
    parser.add_argument("--revision", default="0local")
    parser.add_argument("--profile", choices=["release-lite", "release"], default="release-lite")
    parser.add_argument("--binary", type=Path, help="Reuse a binary built on this Fedora distribution")
    parser.add_argument("--copyright-file", type=Path, help="Project-approved copyright/license notice")
    parser.add_argument("--license-expression", default="LicenseRef-Unspecified", help="Project-approved SPDX license expression")
    parser.add_argument("--output-dir", type=Path, default=ROOT / "dist-dev")
    parser.add_argument("--check", action="store_true", help="Run the extracted packaged app on a desktop with Deno")
    args = parser.parse_args()
    validate_metadata(parser, args, "rpm")
    if not re.fullmatch(r"[A-Za-z0-9.+() -]+", args.license_expression):
        parser.error("invalid license expression")
    if not is_development(args.revision) and args.license_expression == "LicenseRef-Unspecified":
        parser.error("a production revision requires --license-expression approved by the project")
    for tool in ["rpmbuild", "rpm"] + (["rpm2cpio", "cpio"] if args.check else []):
        if not shutil.which(tool):
            parser.error(f"missing {tool}; install Fedora's rpm-build and cpio packages")
    binary, version, required_glibc = build_binary(parser, args)
    (ROOT / "build").mkdir(exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="optchat-rpm.", dir=ROOT / "build") as temporary:
        temporary = Path(temporary)
        payload = temporary / "payload"
        for folder in ["usr/bin", "usr/libexec/optchat", "usr/share/applications", "usr/share/icons/hicolor/512x512/apps", "usr/share/doc/optchat", "usr/share/licenses/optchat"]:
            (payload / folder).mkdir(parents=True)
        installed = payload / "usr/libexec/optchat/optchat"
        shutil.copyfile(binary, installed)
        (payload / "usr/bin/optchat").symlink_to("../libexec/optchat/optchat")
        desktop = ROOT / "shells/linux/packaging/com.reframe.optchat.gpui.desktop"
        subprocess.run(["desktop-file-validate", str(desktop)], check=True)
        shutil.copyfile(desktop, payload / "usr/share/applications" / desktop.name)
        icon = ROOT / "shells/linux/packaging/assets/com.reframe.optchat.gpui.png"
        shutil.copyfile(icon, payload / "usr/share/icons/hicolor/512x512/apps" / icon.name)
        copy_notices(payload / "usr/share/licenses/optchat")
        copy_manual(payload / "usr/share/man/man1", version)
        if args.copyright_file:
            shutil.copyfile(args.copyright_file, payload / "usr/share/licenses/optchat/COPYRIGHT")
        (payload / "usr/share/doc/optchat/README").write_text(
            "OptChat is a native client. Add your engine's scoped client token in Workspaces.\n"
            "Preferences use XDG_CONFIG_HOME/optchat or ~/.config/optchat.\n"
            "Tokens use the desktop Secret Service. Removing the package preserves user data.\n"
        )
        for path in payload.rglob("*"):
            if not path.is_symlink():
                path.chmod(0o755 if path.is_dir() else 0o644)
        installed.chmod(0o755)
        topdir = temporary / "rpmbuild"
        for folder in ["BUILD", "BUILDROOT", "RPMS", "SOURCES", "SPECS", "SRPMS"]:
            (topdir / folder).mkdir(parents=True)
        with tarfile.open(topdir / "SOURCES/payload.tar.gz", "w:gz") as archive:
            archive.add(payload / "usr", arcname="usr")
        spec = (ROOT / "shells/linux/packaging/optchat.spec.in").read_text()
        for key, value in {"VERSION": version, "REVISION": args.revision, "LICENSE": args.license_expression,
                           "MAINTAINER": args.maintainer, "CHANGELOG_DATE": datetime.now(timezone.utc).strftime("%a %b %d %Y")}.items():
            spec = spec.replace(f"@{key}@", value)
        spec_path = topdir / "SPECS/optchat.spec"
        spec_path.write_text(spec)
        subprocess.run(["rpmbuild", "--define", f"_topdir {topdir}", "-bb", str(spec_path)], check=True)
        packages = list((topdir / "RPMS").rglob("*.rpm"))
        if len(packages) != 1:
            parser.error("expected exactly one native RPM")
        package = packages[0]
        if args.check:
            extracted = temporary / "extracted"
            extracted.mkdir()
            with subprocess.Popen(["rpm2cpio", str(package)], stdout=subprocess.PIPE) as conversion:
                subprocess.run(["cpio", "-id", "--quiet", "--no-absolute-filenames"], stdin=conversion.stdout, check=True, cwd=extracted)
                conversion.stdout.close()
                if conversion.wait() != 0:
                    raise RuntimeError("rpm2cpio failed")
            deno = os.environ.get("DENO_BIN") or shutil.which("deno") or str(Path.home() / ".deno/bin/deno")
            env = os.environ.copy()
            env["OPTCHAT_TEST_BINARY"] = str(extracted / "usr/bin/optchat")
            env.pop("OPTCHAT_HEADLESS", None)
            subprocess.run([deno, "run", "-A", str(ROOT / "tests/linux.ts")], check=True, cwd=ROOT, env=env)
        report = {"format": "rpm", "maintainer": args.maintainer, "version": command(["rpm", "-qp", "--qf", "%{VERSION}-%{RELEASE}", str(package)]),
                  "architecture": command(["rpm", "-qp", "--qf", "%{ARCH}", str(package)]),
                  "depends": command(["rpm", "-qp", "--requires", str(package)]).splitlines(),
                  "required_glibc": required_glibc, "profile": args.profile, "license_expression": args.license_expression,
                  "packaged_smoke_test": "passed" if args.check else "not_run", "package_signature": "unsigned"}
        publish(package, args.output_dir, report)


if __name__ == "__main__":
    try:
        main()
    except (OSError, RuntimeError, subprocess.CalledProcessError) as error:
        sys.exit(str(error))
