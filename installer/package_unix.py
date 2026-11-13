#!/usr/bin/env python3
"""Native Linux/macOS release packaging, using the Python 3.10+ standard library.

A target must match the host and each executable's ELF/Mach-O header. No root
installation is performed. Temporary work is confined to a fresh private directory
in --dist-dir; artifacts are published only after native format checks succeed.
"""
from __future__ import annotations

import argparse
import contextlib
import gzip
import hashlib
import json
import math
import os
from pathlib import Path
import platform
import plistlib
import re
import shutil
import stat
import struct
import subprocess
import sys
import tarfile
import tempfile
import time
import zipfile

REPO = Path(__file__).resolve().parent.parent
ARCHIVES = {".hpi", ".ufo", ".ccx", ".gp3", ".gpf"}
PROGRAMS = ("ta-launcher", "ta-reborn")
TARGETS = {
    "x86_64-unknown-linux-gnu": ("linux", "x64", "x86_64", "amd64", 62),
    "aarch64-unknown-linux-gnu": ("linux", "arm64", "aarch64", "arm64", 183),
    "x86_64-apple-darwin": ("macos", "x64", "x86_64", "", 0x01000007),
    "aarch64-apple-darwin": ("macos", "arm64", "aarch64", "", 0x0100000C),
}
PROJECT = "https://github.com/Jared-woodruff/TA-Reborn"
MAINTAINER = "TA Reborn contributors <47959206+Jared-woodruff@users.noreply.github.com>"
MIN_MACOS = "13.0"
SEMVER = re.compile(r"[0-9]+\.[0-9]+\.[0-9]+(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?")


def run(args, *, cwd=None, capture=False, env=None):
    result = subprocess.run([str(a) for a in args], cwd=cwd, env=env, check=True,
                            stdout=subprocess.PIPE if capture else None, text=capture)
    return result.stdout if capture else None


def create_disk_image(source, dmg, *, attempts=5, pause=20):
    """`hdiutil create` for the disk image, tried again while the system holds the new image's
    device: on GitHub's macOS runners it fails now and then with "Resource busy" (a scan of the
    new volume still holding it) and succeeds on a later try. Any other failure stops at once."""
    command = [str(a) for a in ("hdiutil", "create", "-volname", "TA Reborn", "-srcfolder", source, "-ov", "-format", "UDZO", dmg)]
    for attempt in range(1, attempts + 1):
        result = subprocess.run(command, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        sys.stdout.write(result.stdout)
        sys.stderr.write(result.stderr)
        if result.returncode == 0:
            return
        if attempt == attempts or "Resource busy" not in result.stdout + result.stderr:
            raise subprocess.CalledProcessError(result.returncode, command, result.stdout, result.stderr)
        print(f"hdiutil: resource busy, trying again in {pause} s ({attempt} of {attempts})", flush=True)
        Path(dmg).unlink(missing_ok=True)
        time.sleep(pause)


def require_tools(names):
    missing = [name for name in names if shutil.which(name) is None]
    if missing:
        raise ValueError("Missing packaging tools: " + ", ".join(missing))


def version_from_manifest(path):
    text = path.read_text(encoding="utf-8")
    section = re.search(r"(?ms)^\[workspace\.package\]\s*\n(.*?)(?=^\[|\Z)", text)
    found = re.search(r'^version\s*=\s*"([^"]+)"', section[1], re.M) if section else None
    if not found or not SEMVER.fullmatch(found[1]):
        raise ValueError("Packaging requires a valid workspace release version")
    return found[1]


def deb_version(version):
    # Debian sorts '~rc' before the final release; keep upstream filenames intact.
    return version.replace("-", "~") + "-1"


def arch_version(version):
    # pkgver may contain '_' and '+', but no hyphens (reserved for pkgrel).
    return version.replace("-", "_") + "-1"


def validate_native(target, package_platform, system=None, machine=None):
    if target not in TARGETS or TARGETS[target][0] != package_platform:
        raise ValueError(f"Unsupported {package_platform} target: {target}")
    system = system or platform.system()
    machine = (machine or platform.machine()).lower()
    host_arch = {"amd64": "x86_64", "arm64": "aarch64"}.get(machine, machine)
    wanted_system = "Linux" if package_platform == "linux" else "Darwin"
    if system != wanted_system or host_arch != TARGETS[target][2]:
        raise ValueError(f"Build {target} on a native {wanted_system}/{TARGETS[target][2]} host, not {system}/{machine}")


def validate_binary(path, target):
    if path.is_symlink() or not path.is_file():
        raise ValueError(f"Missing regular executable: {path}")
    with path.open("rb") as stream:
        header = stream.read(64)
    if TARGETS[target][0] == "linux":
        if len(header) < 64 or header[:6] != b"\x7fELF\x02\x01":
            raise ValueError(f"Expected a little-endian 64-bit ELF executable: {path}")
        machine = struct.unpack_from("<H", header, 18)[0]
        if struct.unpack_from("<H", header, 16)[0] not in (2, 3):
            raise ValueError(f"ELF file is not an executable or PIE: {path}")
    else:
        if len(header) < 32 or header[:4] != b"\xcf\xfa\xed\xfe":
            raise ValueError(f"Expected a native thin 64-bit Mach-O executable: {path}")
        machine = struct.unpack_from("<I", header, 4)[0]
        if struct.unpack_from("<I", header, 12)[0] != 2:
            raise ValueError(f"Mach-O file is not an executable: {path}")
        validate_macos_minimum(path, header)
    if machine != TARGETS[target][4]:
        raise ValueError(f"Executable architecture does not match {target}: {path}")


def validate_macos_minimum(path, header):
    # Inspect the actual executable, including --skip-build inputs, instead of
    # trusting the build environment or a manually written Info.plist minimum.
    ncmds, size = struct.unpack_from("<II", header, 16)
    if not ncmds or ncmds > 65536 or size > 16 * 1024 * 1024:
        raise ValueError(f"Invalid Mach-O load-command table: {path}")
    with path.open("rb") as stream:
        stream.seek(32)
        commands = stream.read(size)
    if len(commands) != size:
        raise ValueError(f"Truncated Mach-O load commands: {path}")
    offset, versions = 0, []
    for _ in range(ncmds):
        if offset + 8 > size:
            raise ValueError(f"Truncated Mach-O load command: {path}")
        command, length = struct.unpack_from("<II", commands, offset)
        if length < 8 or length % 8 or offset + length > size:
            raise ValueError(f"Invalid Mach-O load-command length: {path}")
        if command == 0x32:  # LC_BUILD_VERSION: platform, minos, sdk, ntools.
            if length < 24 or struct.unpack_from("<I", commands, offset + 8)[0] != 1:
                raise ValueError(f"Expected a macOS LC_BUILD_VERSION: {path}")
            versions.append(struct.unpack_from("<I", commands, offset + 12)[0])
        elif command == 0x24:  # Older LC_VERSION_MIN_MACOSX.
            if length < 16:
                raise ValueError(f"Invalid macOS minimum-version command: {path}")
            versions.append(struct.unpack_from("<I", commands, offset + 8)[0])
        offset += length
    if offset != size or not versions:
        raise ValueError(f"Missing or inconsistent macOS minimum-version commands: {path}")
    major, minor = map(int, MIN_MACOS.split("."))
    if max(versions) > (major << 16 | minor << 8):
        raise ValueError(f"Executable requires macOS newer than {MIN_MACOS}: {path}")


def safe_tree(path):
    """Reject linked/special files and original-game archives before copying/packing."""
    if path.is_symlink():
        raise ValueError(f"Linked package input is not allowed: {path}")
    for item in [path, *path.rglob("*")] if path.is_dir() else [path]:
        mode = item.lstat().st_mode
        if not (stat.S_ISDIR(mode) or stat.S_ISREG(mode)):
            raise ValueError(f"Linked or special package input is not allowed: {item}")
        if item.suffix.lower() in ARCHIVES:
            raise ValueError(f"Original game archives must not be distributed: {item}")


def internal_docs():
    """The documents in docs/ that stay in the repository (internal reviews, QA notes, the release
    process, docs/media): installer/files/internal_docs.txt, shared with the Windows setup."""
    lines = (REPO / "installer/files/internal_docs.txt").read_text(encoding="utf-8").splitlines()
    return tuple(line.strip() for line in lines if line.strip() and not line.startswith("#"))


def copy_tree(source, destination, *, skip=()):
    safe_tree(source)
    if not source.is_dir():
        raise ValueError(f"Missing resource directory: {source}")
    shutil.copytree(source, destination, ignore=shutil.ignore_patterns(*skip))


def write_text(path, text, mode=0o644):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text, encoding="utf-8", newline="\n")
    path.chmod(mode)


def copy_file(source, destination, mode=0o644):
    safe_tree(source)
    destination.parent.mkdir(parents=True, exist_ok=True)
    shutil.copy2(source, destination)
    destination.chmod(mode)


def normalise_modes(root):
    for path in [root, *root.rglob("*")]:
        if path.is_symlink():
            raise ValueError(f"Unexpected package symlink: {path}")
        if path.is_dir():
            path.chmod(0o755)
        else:
            path.chmod(0o755 if path.stat().st_mode & 0o111 else 0o644)


@contextlib.contextmanager
def staging(dist, name):
    dist.mkdir(parents=True, exist_ok=True)
    dist = dist.resolve(strict=True)
    root = Path(tempfile.mkdtemp(prefix=f".{name}-", dir=dist))
    try:
        yield root
    finally:
        # Remove only this invocation's unique directory, never a reused stage.
        if root.is_symlink() or root.resolve().parent != dist or not root.name.startswith(f".{name}-"):
            raise ValueError(f"Refusing unsafe staging cleanup: {root}")
        shutil.rmtree(root)


def sha256(path):
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def tar_gz(source, output, arcname=None):
    safe_tree(source)
    with tarfile.open(output, "w:gz", compresslevel=6) as archive:
        def owner(info):
            info.uid = info.gid = 0
            info.uname = info.gname = "root"
            return info
        archive.add(source, arcname=arcname or source.name, filter=owner)
    with tarfile.open(output, "r:gz") as archive:
        if not archive.getmembers():
            raise ValueError(f"Empty package: {output}")
        # Read the complete compressed stream, checking content against staging.
        prefix = arcname or source.name
        for member in archive:
            if member.isfile():
                relative = Path(member.name).relative_to(prefix)
                with archive.extractfile(member) as stream:
                    if stream_sha256(stream) != sha256(source / relative):
                        raise ValueError(f"Archive payload mismatch: {member.name}")


def tree_hashes(root, exclude=()):
    safe_tree(root)
    return {p.relative_to(root).as_posix(): sha256(p) for p in root.rglob("*")
            if p.is_file() and not any(part in exclude for part in p.relative_to(root).parts)}


def verify_tree(expected, actual, exclude=()):
    if tree_hashes(expected, exclude) != tree_hashes(actual, exclude):
        raise ValueError(f"Packaged files differ from staging: {actual}")


def verify_app_zip(app, zipped):
    expected = {app.name + "/" + name: value for name, value in tree_hashes(app).items()}
    with zipfile.ZipFile(zipped) as archive:
        if archive.testzip() is not None:
            raise ValueError("Corrupt macOS application ZIP")
        actual = {}
        for entry in archive.infolist():
            if entry.is_dir() or entry.filename.startswith("__MACOSX/"):
                continue
            if entry.filename in actual:
                raise ValueError(f"Duplicate macOS ZIP entry: {entry.filename}")
            with archive.open(entry) as stream:
                actual[entry.filename] = stream_sha256(stream)
        if expected != actual:
            raise ValueError("macOS ZIP contents differ from the signed app")


def stream_sha256(stream):
    digest = hashlib.sha256()
    for chunk in iter(lambda: stream.read(1024 * 1024), b""):
        digest.update(chunk)
    return digest.hexdigest()


def stage_resources(root, package_platform, tools, target, metadata):
    root.mkdir(parents=True, exist_ok=True)
    for directory in ("assets", "lang", "docs"):
        copy_tree(REPO / directory, root / directory, skip=internal_docs() if directory == "docs" else ())
    copy_file(REPO / "CHANGELOG.md", root / "docs/CHANGELOG.md")
    copy_file(REPO / "LICENSE", root / "LICENSE.txt")
    copy_file(REPO / f"installer/{package_platform}/README.txt", root / "README.txt")
    copy_file(REPO / "installer/files/mods/README.txt", root / "mods/README.txt")
    run([tools, "notices", root / "THIRD-PARTY-NOTICES.txt", "--target", target], cwd=REPO)
    if not (root / "THIRD-PARTY-NOTICES.txt").is_file():
        raise ValueError("ta-tools did not create third-party notices")
    write_text(root / "build-info.json", json.dumps(metadata, indent=2) + "\n")
    safe_tree(root)


def metadata_for(target, version):
    return {"version": version, "target": target,
            "commit": run(["git", "rev-parse", "HEAD"], cwd=REPO, capture=True).strip(),
            "dirty": bool(run(["git", "status", "--porcelain"], cwd=REPO, capture=True).strip()),
            "rustc": run(["rustc", "-Vv"], capture=True).strip(),
            "built_unix": int(os.environ.get("SOURCE_DATE_EPOCH", time.time())),
            "platform": platform.platform(), "original_game_data_included": False}


def linux_system_tree(portable, root):
    copy_tree(portable, root / "opt/ta-reborn")
    (root / "opt/ta-reborn/install.sh").unlink()
    for name in PROGRAMS:
        write_text(root / "usr/bin" / name, f'#!/bin/sh\nexec /opt/ta-reborn/{name} "$@"\n', 0o755)
    copy_file(REPO / "installer/linux/ta-reborn.desktop", root / "usr/share/applications/ta-reborn-launcher.desktop")
    for size in (16, 24, 32, 48, 64, 128, 256, 512):
        copy_file(REPO / f"branding/emblem-{size}.png", root / f"usr/share/icons/hicolor/{size}x{size}/apps/ta-reborn.png")
    copy_file(REPO / "LICENSE", root / "usr/share/licenses/ta-reborn/LICENSE")
    copy_file(REPO / "LICENSE", root / "usr/share/doc/ta-reborn/copyright")
    normalise_modes(root)
    run(["desktop-file-validate", root / "usr/share/applications/ta-reborn-launcher.desktop"])


def deb_control(version, arch, root):
    size = sum(max(1, math.ceil(p.stat().st_size / 1024)) if p.is_file() else 1 for p in root.rglob("*"))
    return f"""Package: ta-reborn
Version: {deb_version(version)}
Section: games
Priority: optional
Architecture: {arch}
Maintainer: {MAINTAINER}
Homepage: {PROJECT}
Installed-Size: {size}
Depends: libc6 (>= 2.35), libgcc-s1, libstdc++6, libasound2, libudev1, libx11-6, libx11-xcb1, libxcursor1, libxi6, libxrandr2, libxkbcommon0, libxkbcommon-x11-0, libwayland-client0, libwayland-cursor0, libegl1, libgl1, libvulkan1, libarchive-tools, curl, xdg-utils
Recommends: xdg-desktop-portal, xdg-desktop-portal-gtk
Description: Unofficial Total Annihilation engine and launcher
 Native real-time strategy engine with the TA Reborn launcher.
 Requires data from your own copy of Total Annihilation.
 No original game data is included.
"""


def arch_pkginfo(version, arch, size, epoch):
    deps = ["glibc>=2.35", "gcc-libs", "alsa-lib", "systemd-libs", "libx11", "libxcursor",
            "libxi", "libxrandr", "libxkbcommon", "libxkbcommon-x11", "wayland", "libglvnd", "vulkan-icd-loader", "libarchive", "curl", "xdg-utils"]
    lines = ["pkgname = ta-reborn", "pkgbase = ta-reborn", "xdata = pkgtype=pkg", f"pkgver = {arch_version(version)}",
             "pkgdesc = Unofficial Total Annihilation engine and launcher; requires original game data",
             f"url = {PROJECT}", f"builddate = {epoch}", f"packager = {MAINTAINER}", f"size = {size}",
             f"arch = {arch}", "license = GPL-3.0-or-later"]
    lines += [f"depend = {dep}" for dep in deps]
    lines += ["optdepend = xdg-desktop-portal: desktop file dialogs",
              "optdepend = xdg-desktop-portal-gtk: GTK portal backend",
              "optdepend = vulkan-driver: hardware Vulkan rendering"]
    return "\n".join(lines) + "\n"


def arch_buildinfo(version, arch, epoch, work):
    # BUILDINFO v2 permits zero installed/buildenv/options entries. This is a
    # Debian-hosted native Cargo build, so do not invent Arch installed packages
    # or makepkg.conf settings. build-info.json records the actual dpkg inventory.
    recipe = REPO / "installer/linux/PKGBUILD"
    return f"""format = 2
pkgname = ta-reborn
pkgbase = ta-reborn
pkgver = {arch_version(version)}
pkgarch = {arch}
pkgbuild_sha256sum = {sha256(recipe)}
packager = {MAINTAINER}
builddate = {epoch}
builddir = {work.as_posix()}
startdir = {REPO.as_posix()}
buildtool = ta-reborn-package-unix
buildtoolver = {arch_version(version)}-{arch}
# Native Cargo build and binary payload recipe on the recorded Linux host.
# Not built with makepkg; see /opt/ta-reborn/build-info.json for dpkg inventory.
"""


def stage_arch_tree(source, destination, version):
    safe_tree(source)
    destination.mkdir()
    environment = os.environ.copy()
    environment.update(TA_REBORN_PAYLOAD=source.as_posix(), pkgdir=destination.as_posix(),
                       TA_REBORN_PKGVER=arch_version(version).rsplit("-", 1)[0])
    run(["bash", "-euc", 'source "$1"; package', "ta-reborn-packager", REPO / "installer/linux/PKGBUILD"],
        cwd=REPO, env=environment)
    verify_tree(source, destination)


def linux_build_environment():
    output = run(["dpkg-query", "--show", "--showformat=${db:Status-Status}\t${binary:Package}\t${Version}\t${Architecture}\n"], capture=True)
    packages = []
    for line in output.splitlines():
        fields = line.split("\t")
        if len(fields) == 4 and fields[0] == "installed":
            packages.append(dict(name=fields[1], version=fields[2], architecture=fields[3]))
    if not packages:
        raise ValueError("Could not record the native Debian/Ubuntu build environment")
    return {"os_release": Path("/etc/os-release").read_text(encoding="utf-8"),
            "package_manager": "dpkg", "installed_packages": packages,
            "packager": "installer/package_unix.py", "packager_sha256": sha256(REPO / "installer/package_unix.py"),
            "arch_payload_recipe": "installer/linux/PKGBUILD", "arch_payload_recipe_sha256": sha256(REPO / "installer/linux/PKGBUILD")}


def mtree_bytes(root):
    # ALPM .MTREE is gzip-compressed text; nonportable path bytes use octal escapes.
    lines = ["#mtree", "/set uid=0 gid=0"]
    for path in sorted(root.rglob("*")):
        if path.name == ".MTREE":
            continue
        raw = ("./" + path.relative_to(root).as_posix()).encode("utf-8")
        escaped = "".join(chr(byte) if 33 <= byte <= 126 and byte not in (35, 61, 92) else f"\\{byte:03o}" for byte in raw)
        info = path.stat()
        line = f"{escaped} type={'dir' if path.is_dir() else 'file'} mode={stat.S_IMODE(info.st_mode):o} time={int(info.st_mtime)}"
        if path.is_file():
            line += f" size={info.st_size} sha256digest={sha256(path)}"
        lines.append(line)
    return gzip.compress(("\n".join(lines) + "\n").encode("ascii"), mtime=0)


def arch_members(root):
    # libalpm compares these metadata entry names literally. Adding the tree as
    # '.' produces './.PKGINFO', which bsdtar can read but pacman cannot install.
    names = sorted(path.name for path in root.iterdir())
    if not {".PKGINFO", ".MTREE", ".BUILDINFO"} <= set(names):
        raise ValueError("Arch package metadata is missing")
    return names


def linux_packages(work, release, name, target, version, metadata):
    require_tools(["objcopy", "strip", "readelf", "ldd", "dpkg-deb", "dpkg-query", "bash", "bsdtar", "zstd", "desktop-file-validate"])
    metadata["linux_build_environment"] = linux_build_environment()
    portable = work / name
    stage_resources(portable, "linux", release / "ta-tools", target, metadata)
    symbols = work / f"{name}-symbols"
    symbols.mkdir()
    for binary in PROGRAMS:
        copy_file(release / binary, portable / binary, 0o755)
        versions = run(["readelf", "--version-info", portable / binary], capture=True)
        needed = [tuple(map(int, value.split("."))) for value in re.findall(r"GLIBC_([0-9]+\.[0-9]+)", versions)]
        if needed and max(needed) > (2, 35):
            raise ValueError(f"{binary} requires glibc newer than 2.35; rebuild on Ubuntu 22.04")
        run(["objcopy", "--only-keep-debug", portable / binary, symbols / f"{binary}.debug"])
        run(["strip", "--strip-unneeded", portable / binary])
        run(["objcopy", f"--add-gnu-debuglink={symbols / (binary + '.debug')}", portable / binary])
        validate_binary(portable / binary, target)
        dependencies = run(["ldd", portable / binary], capture=True)
        if "not found" in dependencies:
            raise ValueError(f"Unresolved native runtime dependencies for {binary}:\n{dependencies}")
        run(["readelf", "--string-dump=.gnu_debuglink", portable / binary])
        copy_file(portable / binary, symbols / binary, 0o755)
    copy_file(REPO / "branding/emblem-512.png", portable / "ta-reborn.png")
    copy_file(REPO / "installer/linux/install.sh", portable / "install.sh", 0o755)
    copy_file(portable / "build-info.json", symbols / "build-info.json")
    normalise_modes(portable)
    tar_gz(portable, work / f"{name}.tar.gz")
    tar_gz(symbols, work / f"{name}-symbols.tar.gz")
    system = work / "linux-root"
    linux_system_tree(portable, system)
    write_text(system / "DEBIAN/control", deb_control(version, TARGETS[target][3], system))
    deb = work / f"{name}-ubuntu.deb"
    run(["dpkg-deb", "--root-owner-group", "--build", system, deb])
    if run(["dpkg-deb", "--field", deb, "Architecture"], capture=True).strip() != TARGETS[target][3]:
        raise ValueError("Debian package architecture mismatch")
    run(["dpkg-deb", "--contents", deb], capture=True)
    checked_deb = work / "checked-deb"
    run(["dpkg-deb", "--extract", deb, checked_deb])
    verify_tree(system, checked_deb, exclude=("DEBIAN",))
    shutil.rmtree(system / "DEBIAN")  # fixed child of this invocation's private tree
    arch_root = work / "arch-root"
    stage_arch_tree(system, arch_root, version)
    system = arch_root
    size = sum(p.stat().st_size for p in system.rglob("*") if p.is_file())
    write_text(system / ".PKGINFO", arch_pkginfo(version, TARGETS[target][2], size, metadata["built_unix"]))
    write_text(system / ".BUILDINFO", arch_buildinfo(version, TARGETS[target][2], metadata["built_unix"], work))
    (system / ".MTREE").write_bytes(mtree_bytes(system))
    arch = work / f"{name}-arch.pkg.tar.zst"
    run(["bsdtar", "--zstd", "--uid", "0", "--gid", "0", "--uname", "root", "--gname", "root",
         "--no-xattrs", "-cf", arch, "-C", system, *arch_members(system)])
    run(["zstd", "--test", arch])
    members = set(run(["bsdtar", "-tf", arch], capture=True).splitlines())
    if not {".PKGINFO", ".MTREE", ".BUILDINFO"} <= members or any(name.startswith("./") for name in members):
        raise ValueError("Arch archive metadata must use unprefixed root entry names")
    pkginfo = run(["bsdtar", "-xOf", arch, ".PKGINFO"], capture=True)
    if f"arch = {TARGETS[target][2]}\n" not in pkginfo:
        raise ValueError("Arch package architecture mismatch")
    checked_arch = work / "checked-arch"
    checked_arch.mkdir()
    run(["bsdtar", "-xf", arch, "-C", checked_arch])
    verify_tree(system, checked_arch)
    return [work / f"{name}.tar.gz", deb, arch, work / f"{name}-symbols.tar.gz"]


def macos_plist(version, arch):
    numeric = re.match(r"[0-9]+\.[0-9]+\.[0-9]+", version)[0]
    return {"CFBundleDevelopmentRegion": "en", "CFBundleDisplayName": "TA Reborn", "CFBundleName": "TA Reborn",
            "CFBundleIdentifier": "io.github.jared-woodruff.ta-reborn", "CFBundleExecutable": "ta-launcher",
            "CFBundleIconFile": "ta-reborn.icns", "CFBundleInfoDictionaryVersion": "6.0", "CFBundlePackageType": "APPL",
            "CFBundleShortVersionString": numeric, "CFBundleVersion": numeric,
            "CFBundleGetInfoString": f"TA Reborn {version}", "TARebornVersion": version, "LSMinimumSystemVersion": MIN_MACOS,
            "LSArchitecturePriority": ["arm64" if arch == "arm64" else "x86_64"], "NSHighResolutionCapable": True,
            "NSHumanReadableCopyright": "Total Annihilation is © its respective owners. TA Reborn is GPL-3.0-or-later."}


def macos_uuids(text):
    values = {arch: value.lower() for value, arch in re.findall(r"UUID: ([0-9A-Fa-f-]{36}) \(([^)]+)\)", text)}
    if not values:
        raise ValueError("Missing executable/dSYM UUID")
    return values


def sign_macos_app(app, identity):
    signing = ["--force", "--sign", identity]
    if identity != "-":
        signing += ["--options", "runtime", "--timestamp"]
    # codesign can recognize the main executable as its enclosing bundle and
    # validate nested code immediately. Sign the companion first, even when the
    # linker did not supply an ad-hoc signature (as on Intel).
    for binary in ("ta-reborn", "ta-launcher"):
        run(["codesign", *signing, app / "Contents/MacOS" / binary])
    run(["codesign", *signing, app])
    run(["codesign", "--verify", "--deep", "--strict", "--verbose=2", app])


def macos_packages(work, release, name, target, version, metadata):
    require_tools(["iconutil", "dsymutil", "strip", "codesign", "ditto", "hdiutil", "otool", "plutil", "xcrun"])
    identity = os.environ.get("MACOS_SIGN_IDENTITY", "-").strip() or "-"
    notary = os.environ.get("MACOS_NOTARY_PROFILE", "").strip()
    if notary and identity == "-":
        raise ValueError("Notarization requires MACOS_SIGN_IDENTITY (Developer ID), not ad-hoc signing")
    metadata["signing"] = "Developer ID; notarization requested" if notary else ("ad-hoc; not notarized" if identity == "-" else "Developer ID; not notarized")
    app = work / "TA Reborn.app"
    resources = app / "Contents/Resources"
    executables = app / "Contents/MacOS"
    stage_resources(resources, "macos", release / "ta-tools", target, metadata)
    executables.mkdir(parents=True)
    symbols = work / f"{name}-symbols"
    symbols.mkdir()
    for binary in PROGRAMS:
        copy_file(release / binary, executables / binary, 0o755)
        # Generate dSYM before stripping/signing; its UUID matches this build.
        run(["dsymutil", release / binary, "-o", symbols / f"{binary}.dSYM"])
        if not (symbols / f"{binary}.dSYM/Contents/Resources/DWARF" / binary).is_file():
            raise ValueError(f"Missing dSYM for {binary}")
        run(["strip", "-S", executables / binary])
        validate_binary(executables / binary, target)
        linked = run(["otool", "-L", executables / binary], capture=True)
        for line in linked.splitlines()[1:]:
            dependency = line.strip().split(" (", 1)[0]
            if not dependency.startswith(("/System/Library/", "/usr/lib/")):
                raise ValueError(f"Unbundled macOS dependency for {binary}: {dependency}")
    with (app / "Contents/Info.plist").open("wb") as stream:
        plistlib.dump(macos_plist(version, TARGETS[target][1]), stream)
    write_text(app / "Contents/PkgInfo", "APPL????")
    icons = work / "ta-reborn.iconset"
    icons.mkdir()
    for size in (16, 32, 128, 256, 512):
        for scale in (1, 2):
            pixels = size * scale
            output = icons / f"icon_{size}x{size}{'@2x' if scale == 2 else ''}.png"
            source = REPO / ("installer/macos/emblem-1024.png" if pixels == 1024 else f"branding/emblem-{pixels}.png")
            # The 1024px representation is rendered from the original emblem.svg,
            # not enlarged from 512px: resvg --width 1024 --height 1024 branding/emblem.svg installer/macos/emblem-1024.png
            copy_file(source, output)
    run(["iconutil", "-c", "icns", "-o", resources / "ta-reborn.icns", icons])
    run(["plutil", "-lint", app / "Contents/Info.plist"])
    normalise_modes(app)
    sign_macos_app(app, identity)
    for binary in PROGRAMS:
        executable_uuid = macos_uuids(run(["xcrun", "dwarfdump", "--uuid", executables / binary], capture=True))
        symbols_uuid = macos_uuids(run(["xcrun", "dwarfdump", "--uuid", symbols / f"{binary}.dSYM"], capture=True))
        if executable_uuid != symbols_uuid:
            raise ValueError(f"dSYM UUID does not match the packaged executable: {binary}")
    zipped = work / f"{name}.zip"
    run(["ditto", "-c", "-k", "--sequesterRsrc", "--keepParent", app, zipped])
    if notary:
        run(["xcrun", "notarytool", "submit", zipped, "--keychain-profile", notary, "--wait"])
        run(["xcrun", "stapler", "staple", app])
        run(["xcrun", "stapler", "validate", app])
        # Signed metadata records the request; modifying it after notarization would
        # invalidate the signature. A failed submission never publishes artifacts.
        zipped.unlink()
        run(["ditto", "-c", "-k", "--sequesterRsrc", "--keepParent", app, zipped])
    verify_app_zip(app, zipped)
    dmg_root = work / "disk-image"
    dmg_root.mkdir()
    run(["ditto", app, dmg_root / app.name])
    (dmg_root / "Applications").symlink_to("/Applications")
    copy_file(REPO / "installer/macos/README.txt", dmg_root / "README.txt")
    dmg = work / f"{name}.dmg"
    create_disk_image(dmg_root, dmg)
    if identity != "-":
        run(["codesign", "--force", "--sign", identity, "--timestamp", dmg])
    if notary:
        run(["xcrun", "notarytool", "submit", dmg, "--keychain-profile", notary, "--wait"])
        run(["xcrun", "stapler", "staple", dmg])
        run(["xcrun", "stapler", "validate", dmg])
    run(["hdiutil", "verify", dmg])
    for binary in PROGRAMS:
        copy_file(executables / binary, symbols / binary, 0o755)
    copy_file(resources / "build-info.json", symbols / "build-info.json")
    tar_gz(symbols, work / f"{name}-symbols.tar.gz")
    return [zipped, dmg, work / f"{name}-symbols.tar.gz"]


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("platform", choices=("linux", "macos"))
    parser.add_argument("--target", help="Native Rust target triple (defaults to rustc's host)")
    parser.add_argument("--target-dir", default=os.environ.get("CARGO_TARGET_DIR", "target"), help="Cargo output root, relative to repository or absolute")
    parser.add_argument("--dist-dir", default="dist", help="Package output directory, relative to repository or absolute")
    parser.add_argument("--skip-build", action="store_true", help="Reuse target/<triple>/release binaries already built for this host")
    args = parser.parse_args(argv)
    require_tools(["cargo", "rustc", "git"])
    target = args.target
    if not target:
        target = next((line[6:] for line in run(["rustc", "-vV"], capture=True).splitlines() if line.startswith("host: ")), "")
    validate_native(target, args.platform)
    version = version_from_manifest(REPO / "Cargo.toml")
    name = f"TA-Reborn-{version}-{args.platform}-{TARGETS[target][1]}"
    target_dir = (REPO / args.target_dir).resolve()
    dist = (REPO / args.dist_dir).resolve()
    release = target_dir / target / "release"
    if args.platform == "macos":
        deployment = os.environ.get("MACOSX_DEPLOYMENT_TARGET", MIN_MACOS)
        if deployment != MIN_MACOS:
            raise ValueError(f"MACOSX_DEPLOYMENT_TARGET must be {MIN_MACOS} to match the published minimum")
    if not args.skip_build:
        environment = os.environ.copy()
        if args.platform == "macos": environment["MACOSX_DEPLOYMENT_TARGET"] = MIN_MACOS
        run(["cargo", "build", "--release", "--locked", "--target", target, "--target-dir", target_dir,
             "-p", "ta_game", "-p", "ta_launcher", "-p", "ta_tools"], cwd=REPO, env=environment)
    for binary in (*PROGRAMS, "ta-tools"):
        validate_binary(release / binary, target)
    metadata = metadata_for(target, version)
    with staging(dist, name) as work:
        artifacts = (linux_packages if args.platform == "linux" else macos_packages)(work, release, name, target, version, metadata)
        sums = work / f"{name}-SHA256SUMS.txt"
        write_text(sums, "".join(f"{sha256(path)}  {path.name}\n" for path in artifacts))
        outputs = [*artifacts, sums]
        # Preflight every destination before replacing any previous artifact.
        for source in outputs:
            destination = dist / source.name
            if destination.is_symlink() or (destination.exists() and not destination.is_file()):
                raise ValueError(f"Refusing to replace non-regular output: {destination}")
        for source in outputs:
            destination = dist / source.name
            os.replace(source, destination)
            print(destination)


if __name__ == "__main__":
    try:
        main()
    except (ValueError, OSError, subprocess.CalledProcessError) as error:
        print(f"Packaging failed: {error}", file=sys.stderr)
        raise SystemExit(1)
