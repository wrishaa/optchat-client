#!/usr/bin/env python3
"""Build a native Debian package; determine ABI dependencies on the build distribution."""
import argparse
from datetime import datetime, timezone
from email.utils import format_datetime
import gzip
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
from package_common import ROOT, build_binary, command, copy_manual, copy_notices, default_maintainer, publish, validate_metadata


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--maintainer", default=default_maintainer(), help="Package maintainer; overrides OPTCHAT_PACKAGE_MAINTAINER and the approved default")
    parser.add_argument("--revision", default="0local", help="Debian revision; use 1 for the first production revision")
    parser.add_argument("--profile", choices=["release-lite", "release"], default="release-lite")
    parser.add_argument("--binary", type=Path, help="Reuse a binary built on this same distribution")
    parser.add_argument("--copyright-file", type=Path, help="Project-approved copyright/license notice")
    parser.add_argument("--output-dir", type=Path, default=ROOT / "dist-dev")
    parser.add_argument("--check", action="store_true", help="Test the extracted packaged executable with the fake engine")
    args = parser.parse_args()
    validate_metadata(parser, args, "deb")
    for tool in ["dpkg", "dpkg-deb", "dpkg-shlibdeps", "desktop-file-validate", "readelf"]:
        if not shutil.which(tool):
            parser.error(f"missing {tool}; install dpkg-dev, desktop-file-utils and binutils")
    architecture = command(["dpkg", "--print-architecture"])
    if architecture not in ["amd64", "arm64"]:
        parser.error("only native amd64 and arm64 builds are configured")
    binary, version, required_glibc = build_binary(parser, args)
    (ROOT / "build").mkdir(exist_ok=True)
    deb_version = f"{version}-{args.revision}"
    name = f"optchat_{deb_version}_{architecture}.deb"
    with tempfile.TemporaryDirectory(prefix="optchat-deb.", dir=ROOT / "build") as temporary:
        temporary = Path(temporary)
        package = temporary / "package"
        for folder in ["DEBIAN", "usr/bin", "usr/lib/optchat", "usr/share/applications", "usr/share/icons/hicolor/512x512/apps", "usr/share/doc/optchat/font-licenses"]:
            (package / folder).mkdir(parents=True, exist_ok=True)
        installed_binary = package / "usr/lib/optchat/optchat"
        shutil.copyfile(binary, installed_binary)
        installed_binary.chmod(0o755)
        (package / "usr/bin/optchat").symlink_to("../lib/optchat/optchat")
        desktop = ROOT / "shells/linux/packaging/com.reframe.optchat.gpui.desktop"
        subprocess.run(["desktop-file-validate", str(desktop)], check=True)
        shutil.copyfile(desktop, package / "usr/share/applications" / desktop.name)
        icon = ROOT / "shells/linux/packaging/assets/com.reframe.optchat.gpui.png"
        shutil.copyfile(icon, package / "usr/share/icons/hicolor/512x512/apps" / icon.name)
        copy_notices(package / "usr/share/doc/optchat/font-licenses")
        copy_manual(package / "usr/share/man/man1", version, compressed=True)
        changelog = (
            f"optchat ({deb_version}) UNRELEASED; urgency=medium\n\n"
            "  * Build the native OptChat Linux client package.\n\n"
            f" -- {args.maintainer}  {format_datetime(datetime.now(timezone.utc))}\n"
        )
        (package / "usr/share/doc/optchat/changelog.Debian.gz").write_bytes(gzip.compress(changelog.encode(), mtime=0))
        if args.copyright_file:
            shutil.copyfile(args.copyright_file, package / "usr/share/doc/optchat/copyright")
        (package / "usr/share/doc/optchat/README.Debian").write_text(
            "OptChat is a native client. Add your engine's scoped client token in Workspaces.\n"
            "Preferences use XDG_CONFIG_HOME/optchat or ~/.config/optchat.\n"
            "Client tokens use the desktop Secret Service. Removal preserves user data.\n"
            "Source and application licensing information: https://github.com/fabihatasneem/optchat-client\n"
        )
        # dpkg-shlibdeps needs a source control file, kept in the temporary build directory.
        (temporary / "debian").mkdir()
        (temporary / "debian/control").write_text(f"Source: optchat\nSection: net\nPriority: optional\nMaintainer: {args.maintainer}\n\nPackage: optchat\nArchitecture: any\nDescription: Native OptChat client\n")
        inferred = command(["dpkg-shlibdeps", "-O", "-e" + str(installed_binary)], cwd=temporary)
        dependencies = next(line.split("=", 1)[1] for line in inferred.splitlines() if line.startswith("shlibs:Depends="))
        # Libraries used via dlopen and desktop facilities are not all visible to shlibdeps.
        dependencies += ", libfontconfig1, libwayland-client0, libvulkan1, libegl1, libgles2, ca-certificates, xdg-utils, xdg-desktop-portal"
        installed_kib = sum(p.stat().st_size for p in package.rglob("*") if p.is_file() and not p.is_symlink()) // 1024 + 1
        (package / "DEBIAN/control").write_text(
            f"Package: optchat\nVersion: {deb_version}\nArchitecture: {architecture}\n"
            f"Maintainer: {args.maintainer}\nSection: net\nPriority: optional\nInstalled-Size: {installed_kib}\n"
            f"Depends: {dependencies}\nRecommends: mesa-vulkan-drivers | vulkan-icd, gnome-keyring\n"
            "Homepage: https://github.com/fabihatasneem/optchat-client\n"
            "Description: Native OptChat engine client\n Native Rust/GPUI desktop client with workspaces, tasks and dictation.\n"
        )
        refresh = "#!/bin/sh\nset -e\nif command -v update-desktop-database >/dev/null 2>&1; then update-desktop-database -q || true; fi\nif command -v gtk-update-icon-cache >/dev/null 2>&1; then gtk-update-icon-cache -q -t /usr/share/icons/hicolor || true; fi\n"
        for script in ["postinst", "postrm"]:
            path = package / "DEBIAN" / script
            path.write_text(refresh)
            path.chmod(0o755)
        # Do not inherit the developer's umask into customer-owned system files.
        package.chmod(0o755)
        for path in package.rglob("*"):
            if path.is_symlink():
                continue
            path.chmod(0o755 if path.is_dir() else 0o644)
        installed_binary.chmod(0o755)
        for script in ["postinst", "postrm"]:
            (package / "DEBIAN" / script).chmod(0o755)
        staged_deb = temporary / name
        subprocess.run(["dpkg-deb", "--build", "--root-owner-group", str(package), str(staged_deb)], check=True)
        if args.check:
            extracted = temporary / "extracted"
            subprocess.run(["dpkg-deb", "--extract", str(staged_deb), str(extracted)], check=True)
            deno = os.environ.get("DENO_BIN") or shutil.which("deno") or str(Path.home() / ".deno/bin/deno")
            env = os.environ.copy()
            env["OPTCHAT_TEST_BINARY"] = str(extracted / "usr/bin/optchat")
            env.pop("OPTCHAT_HEADLESS", None)
            subprocess.run([deno, "run", "-A", str(ROOT / "tests/linux.ts")], check=True, cwd=ROOT, env=env)
        report = {"format": "deb", "maintainer": args.maintainer, "version": deb_version, "architecture": architecture, "depends": dependencies,
                  "required_glibc": required_glibc, "profile": args.profile,
                  "packaged_smoke_test": "passed" if args.check else "not_run"}
        publish(staged_deb, args.output_dir, report)


if __name__ == "__main__":
    try:
        main()
    except (OSError, subprocess.CalledProcessError) as error:
        sys.exit(str(error))
