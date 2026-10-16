"""Shared native packaging checks. Production builds use the agreed distribution baselines."""
import hashlib
import gzip
import json
import os
from pathlib import Path
import platform
import re
import shutil
import subprocess

ROOT = Path(__file__).resolve().parents[3]
DEFAULT_MAINTAINER = "Fabiha Tasneem <fabiha@reframe.so>"


def default_maintainer():
    return os.environ.get("OPTCHAT_PACKAGE_MAINTAINER", DEFAULT_MAINTAINER)


def is_development(revision):
    return bool(re.fullmatch(r"0local(?:\.\d+)?", revision))


def command(args, **kwargs):
    return subprocess.check_output(args, text=True, **kwargs).strip()


def validate_metadata(parser, args, package_format):
    if not args.maintainer or not re.fullmatch(r"[^\n<>%]+ <[^\s<>%]+@[^\s<>%]+>", args.maintainer):
        parser.error('provide --maintainer "Name <email>" or OPTCHAT_PACKAGE_MAINTAINER')
    if not re.fullmatch(r"[A-Za-z0-9+.~]+", args.revision):
        parser.error("invalid package revision")
    if os.environ.get("CARGO_BUILD_TARGET"):
        parser.error("build natively on the target distribution/architecture; unset CARGO_BUILD_TARGET")
    if args.copyright_file and not args.copyright_file.is_file():
        parser.error("copyright file does not exist")
    if not is_development(args.revision):
        domain = args.maintainer.rsplit("@", 1)[1].rstrip(">").lower()
        if domain.endswith(".invalid") or domain in ["example.com", "example.org", "example.net"]:
            parser.error("a production revision requires the actual maintainer contact")
        if not args.copyright_file:
            parser.error("a production revision requires --copyright-file with the project's approved notice")
        baseline = ("ubuntu", "24.04") if package_format == "deb" else ("fedora", "43")
        release = platform.freedesktop_os_release()
        if (release.get("ID"), release.get("VERSION_ID")) != baseline:
            parser.error(f"production {package_format} builds must run on {baseline[0]} {baseline[1]}; use container-build.sh")


def build_binary(parser, args):
    for tool in ["desktop-file-validate", "readelf", "git"]:
        if not shutil.which(tool):
            parser.error(f"missing build tool: {tool}")
    cargo = shutil.which("cargo") or str(Path.home() / ".cargo/bin/cargo")
    if not args.binary:
        subprocess.run([str(ROOT / "tools/zed-vendor.sh")], check=True, cwd=ROOT)
    metadata = json.loads(command([cargo, "metadata", "--locked", "--no-deps", "--format-version", "1"], cwd=ROOT))
    version = next(p["version"] for p in metadata["packages"] if p["name"] == "optchat-linux")
    if not re.fullmatch(r"[0-9]+\.[0-9]+\.[0-9]+", version):
        parser.error("native release helpers currently require a numeric major.minor.patch Cargo version")
    if not args.binary:
        subprocess.run([cargo, "build", "--locked", "--profile", args.profile, "-p", "optchat-linux", "-j", os.environ.get("CARGO_BUILD_JOBS", "4")], check=True, cwd=ROOT)
    binary = (args.binary or Path(metadata["target_directory"]) / args.profile / "optchat").resolve()
    machine = platform.machine()
    expected = {"x86_64": "X86-64", "aarch64": "AArch64"}.get(machine)
    if not expected or expected not in command(["readelf", "-h", str(binary)]):
        parser.error("binary architecture must match the native x86_64/ARM64 builder")
    glibc_versions = re.findall(r"\bGLIBC_(\d+(?:\.\d+)+)\b", command(["readelf", "--version-info", str(binary)]))
    required = max(glibc_versions, key=lambda v: tuple(map(int, v.split(".")))) if glibc_versions else None
    host_glibc = os.confstr("CS_GNU_LIBC_VERSION").split()[-1]
    if required and tuple(map(int, required.split("."))) > tuple(map(int, host_glibc.split("."))):
        parser.error(f"binary needs GLIBC {required}, but this builder provides {host_glibc}; rebuild on the baseline")
    return binary, version, required


def copy_notices(destination):
    destination.mkdir(parents=True, exist_ok=True)
    for path in (ROOT / "assets/fonts").glob("*.txt"):
        shutil.copyfile(path, destination / path.name)
    shutil.copyfile(ROOT / "assets/icons/LICENSE-lucide.txt", destination / "LICENSE-lucide.txt")


def copy_manual(destination, version, compressed=False):
    destination.mkdir(parents=True, exist_ok=True)
    manual = (ROOT / "shells/linux/packaging/optchat.1.in").read_text().replace("@VERSION@", version).encode()
    if compressed:
        (destination / "optchat.1.gz").write_bytes(gzip.compress(manual, mtime=0))
    else:
        (destination / "optchat.1").write_bytes(manual)


def publish(package, output_dir, report):
    output_dir = output_dir.resolve()
    output_dir.mkdir(parents=True, exist_ok=True)
    with package.open("rb") as file:
        digest = hashlib.file_digest(file, "sha256").hexdigest()
    report.update({"sha256": digest, "build_os": platform.freedesktop_os_release(),
                   "rustc": command(["rustc", "--version"]),
                   "source_revision": command(["git", "rev-parse", "HEAD"], cwd=ROOT),
                   "source_has_local_changes": bool(command(["git", "status", "--porcelain"], cwd=ROOT)),
                   "install_upgrade_remove_test": "not_run",
                   "builder_image": os.environ.get("OPTCHAT_BUILDER_IMAGE"),
                   "builder_image_id": os.environ.get("OPTCHAT_BUILDER_IMAGE_ID")})
    package.with_name(package.name + ".sha256").write_text(f"{digest}  {package.name}\n")
    package.with_name(package.name + ".build.json").write_text(json.dumps(report, indent=2) + "\n")
    for suffix in ["", ".sha256", ".build.json"]:
        package.with_name(package.name + suffix).replace(output_dir / (package.name + suffix))
    print(f"Built {output_dir / package.name}")
    print("Package creation does not establish clean-desktop install, upgrade, removal or real-engine qualification.")
