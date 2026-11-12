"""Packaging contracts and destructive-operation guards; no cargo/GPU required."""
import ast
import gzip
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import plistlib
import shutil
import struct
import subprocess
import tarfile
import tempfile
import unittest
from unittest import mock
import zipfile


ROOT = Path(__file__).resolve().parents[2]


def load(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


pkg = load("package_unix", ROOT / "installer/package_unix.py")
release = load("release_contract", ROOT / "tools/release/release.py")


def elf(machine=62, kind=3):
    header = bytearray(64)
    header[:6] = b"\x7fELF\x02\x01"
    struct.pack_into("<HH", header, 16, kind, machine)
    return header


def macho(machine=0x01000007, minimum=13 << 16, command=0x32, platform=1):
    header = bytearray(32)
    header[:4] = b"\xcf\xfa\xed\xfe"
    struct.pack_into("<I", header, 4, machine)
    struct.pack_into("<I", header, 12, 2)
    values = (command, 24, platform, minimum, 15 << 16, 0) if command == 0x32 else (command, 16, minimum, 15 << 16)
    body = struct.pack("<" + "I" * len(values), *values)
    struct.pack_into("<II", header, 16, 1, len(body))
    return header + body


class PackagingTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="ta-packaging-test-")
        self.root = Path(self.temp.name)
        self.addCleanup(self.temp.cleanup)

    def file(self, name, data=b"fixture"):
        path = self.root / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(data)
        return path

    def test_native_targets_reject_cross_and_unknown_platforms(self):
        for target, (system, _, machine, _, _) in pkg.TARGETS.items():
            host = "Linux" if system == "linux" else "Darwin"
            pkg.validate_native(target, system, host, machine)
            with self.assertRaisesRegex(ValueError, "native"):
                pkg.validate_native(target, system, host, "aarch64" if machine == "x86_64" else "x86_64")
            with self.assertRaisesRegex(ValueError, "native"):
                pkg.validate_native(target, system, "Windows", machine)
        with self.assertRaisesRegex(ValueError, "Unsupported"):
            pkg.validate_native("x86_64-pc-windows-msvc", "linux", "Linux", "x86_64")

    def test_binary_headers_require_native_architecture_and_executable(self):
        for target, (system, _, _, _, machine) in pkg.TARGETS.items():
            path = self.file(target, elf(machine) if system == "linux" else macho(machine))
            pkg.validate_binary(path, target)
            wrong = elf(183 if machine == 62 else 62) if system == "linux" else macho(0x0100000C if machine == 0x01000007 else 0x01000007)
            path.write_bytes(wrong)
            with self.assertRaisesRegex(ValueError, "architecture"):
                pkg.validate_binary(path, target)
        path = self.file("relocatable", elf(kind=1))
        with self.assertRaisesRegex(ValueError, "not an executable"):
            pkg.validate_binary(path, "x86_64-unknown-linux-gnu")
        path.write_bytes(b"\xca\xfe\xba\xbe" + bytes(60))
        with self.assertRaisesRegex(ValueError, "native thin"):
            pkg.validate_binary(path, "x86_64-apple-darwin")

    def test_macos_minimum_is_checked_in_binary_not_environment(self):
        target = "x86_64-apple-darwin"
        path = self.file("game", macho(minimum=14 << 16))
        with self.assertRaisesRegex(ValueError, "newer than 13.0"):
            pkg.validate_binary(path, target)
        path.write_bytes(macho(minimum=12 << 16, command=0x24))
        pkg.validate_binary(path, target)
        path.write_bytes(macho(platform=2))
        with self.assertRaisesRegex(ValueError, "macOS LC_BUILD_VERSION"):
            pkg.validate_binary(path, target)
        path.write_bytes(macho()[:-1])
        with self.assertRaisesRegex(ValueError, "Truncated"):
            pkg.validate_binary(path, target)
        malformed = macho()
        struct.pack_into("<I", malformed, 36, 0)
        path.write_bytes(malformed)
        with self.assertRaisesRegex(ValueError, "length"):
            pkg.validate_binary(path, target)

    def test_prerelease_keeps_artifact_version_and_native_formats(self):
        manifest = self.file("Cargo.toml", b'[workspace.package]\nversion = "2.1.0-rc.1+build.7"\n[profile.release]\n')
        version = pkg.version_from_manifest(manifest)
        self.assertEqual(version, "2.1.0-rc.1+build.7")
        self.assertEqual(pkg.deb_version(version), "2.1.0~rc.1+build.7-1")
        self.assertEqual(pkg.arch_version(version), "2.1.0_rc.1+build.7-1")
        info = pkg.macos_plist(version, "arm64")
        self.assertEqual(info["CFBundleShortVersionString"], "2.1.0")
        self.assertEqual(info["CFBundleVersion"], "2.1.0")
        self.assertEqual(info["TARebornVersion"], version)
        self.assertIn(version, info["CFBundleGetInfoString"])
        self.assertEqual(plistlib.loads(plistlib.dumps(info)), info)
        for invalid in ("2.1/evil", "2.1.0\nInjected", "../2.1.0", "2.1.0 rc.1"):
            manifest.write_text('[workspace.package]\nversion = "' + invalid + '"\n', encoding="utf-8")
            with self.assertRaises(ValueError):
                pkg.version_from_manifest(manifest)

    def test_distro_metadata_architecture_dependencies_and_prerelease(self):
        for target in ("x86_64-unknown-linux-gnu", "aarch64-unknown-linux-gnu"):
            _, _, arch, deb_arch, _ = pkg.TARGETS[target]
            control = pkg.deb_control("2.1.0-rc.1", deb_arch, self.root)
            self.assertIn("Version: 2.1.0~rc.1-1\n", control)
            self.assertIn(f"Architecture: {deb_arch}\n", control)
            self.assertIn("libarchive-tools", control)
            self.assertIn("libxkbcommon-x11-0", control)
            self.assertIn("libx11-xcb1", control)
            self.assertIn(", curl,", control)
            info = pkg.arch_pkginfo("2.1.0-rc.1", arch, 42, 123)
            self.assertIn("pkgver = 2.1.0_rc.1-1\n", info)
            self.assertIn(f"arch = {arch}\n", info)
            self.assertIn("xdata = pkgtype=pkg\n", info)
            self.assertIn("license = GPL-3.0-or-later\n", info)
            self.assertIn("depend = libarchive\n", info)
            self.assertIn("depend = libxkbcommon-x11\n", info)
            self.assertIn("depend = curl\n", info)

    def test_original_archives_rejected_recursively_case_insensitively(self):
        assets = self.root / "assets"
        self.file("assets/reborn/maps/map.tnt")
        pkg.safe_tree(assets)
        for suffix in pkg.ARCHIVES:
            archive = self.file("assets/reborn/nested/TA" + suffix.upper())
            with self.assertRaisesRegex(ValueError, "Original game archives"):
                pkg.safe_tree(assets)
            archive.unlink()

    def test_symlinks_rejected_in_resources(self):
        target = self.file("private.hpi")
        link = self.root / "assets/harmless.txt"
        link.parent.mkdir()
        try:
            link.symlink_to(target)
        except OSError as error:
            self.skipTest(f"Host cannot create symlinks: {error}")
        with self.assertRaisesRegex(ValueError, "Linked or special"):
            pkg.safe_tree(link.parent)

    def test_staging_cleanup_keeps_unrelated_outputs_on_failure(self):
        keep = self.file("dist/keep.txt")
        with self.assertRaisesRegex(RuntimeError, "fixture failure"):
            with pkg.staging(keep.parent, "TA-Reborn-test") as stage:
                (stage / "work.txt").write_text("temporary", encoding="utf-8")
                raise RuntimeError("fixture failure")
        self.assertTrue(keep.is_file())
        self.assertFalse(stage.exists())

    def test_mtree_hashes_and_escapes_full_utf8_paths(self):
        path = self.file("root/assets/space #=é.txt", b"resource contents")
        output = gzip.decompress(pkg.mtree_bytes(self.root / "root")).decode("ascii")
        self.assertTrue(output.startswith("#mtree\n/set uid=0 gid=0\n"))
        self.assertIn("./assets/space\\040\\043\\075\\303\\251.txt", output)
        self.assertIn("sha256digest=" + hashlib.sha256(path.read_bytes()).hexdigest(), output)

    def test_arch_metadata_uses_exact_root_names_for_libalpm(self):
        root = self.root / "package"
        self.file("package/.PKGINFO")
        self.file("package/.MTREE")
        self.file("package/.BUILDINFO")
        self.file("package/opt/ta-reborn/ta-launcher")
        self.file("package/usr/bin/ta-launcher")
        self.assertEqual(pkg.arch_members(root), [".BUILDINFO", ".MTREE", ".PKGINFO", "opt", "usr"])
        tar = shutil.which("bsdtar") or shutil.which("tar")
        if tar:
            archive = self.root / "fixture.pkg.tar"
            subprocess.run([tar, "-cf", str(archive), "-C", str(root), *pkg.arch_members(root)], check=True)
            with tarfile.open(archive) as packed:
                names = packed.getnames()
                self.assertIn(".PKGINFO", names)
                self.assertIn(".MTREE", names)
                self.assertIn(".BUILDINFO", names)
                self.assertFalse(any(name.startswith("./") for name in names))
        (root / ".PKGINFO").unlink()
        with self.assertRaisesRegex(ValueError, "metadata is missing"):
            pkg.arch_members(root)

    def test_buildinfo_records_actual_recipe_without_inventing_arch_host_packages(self):
        info = pkg.arch_buildinfo("2.1.0-rc.1", "aarch64", 123, self.root)
        self.assertTrue(info.startswith("format = 2\n"))
        self.assertIn("pkgbuild_sha256sum = " + pkg.sha256(ROOT / "installer/linux/PKGBUILD"), info)
        self.assertIn("pkgver = 2.1.0_rc.1-1\n", info)
        self.assertIn("pkgarch = aarch64\n", info)
        self.assertIn("buildtool = ta-reborn-package-unix\n", info)
        self.assertNotIn("\ninstalled = ", info)
        self.assertNotIn("\nbuildenv = ", info)
        self.assertIn("Not built with makepkg", info)

    def test_linux_build_inventory_keeps_debian_versions_and_architectures(self):
        output = "installed\tlibc6:arm64\t2.35-0ubuntu3.10\tarm64\nconfig-files\told-package\t1.0-1\tamd64\ninstalled\tbash\t5.1-6ubuntu1\tarm64\n"
        with mock.patch.object(pkg, "run", return_value=output), mock.patch.object(Path, "read_text", return_value="NAME=Ubuntu\nVERSION_ID=22.04\n"):
            env = pkg.linux_build_environment()
        self.assertEqual(env["package_manager"], "dpkg")
        self.assertEqual(env["installed_packages"], [
            {"name": "libc6:arm64", "version": "2.35-0ubuntu3.10", "architecture": "arm64"},
            {"name": "bash", "version": "5.1-6ubuntu1", "architecture": "arm64"}])
        self.assertIn("Ubuntu", env["os_release"])
        with mock.patch.object(pkg, "run", return_value=""):
            with self.assertRaisesRegex(ValueError, "build environment"):
                pkg.linux_build_environment()

    def test_portable_tar_has_one_root_owned_by_root_and_real_payload(self):
        self.file("TA-Reborn-test/assets/reborn/data.txt", b"original project asset")
        output = self.root / "portable.tar.gz"
        pkg.tar_gz(self.root / "TA-Reborn-test", output)
        with tarfile.open(output, "r:gz") as archive:
            self.assertTrue(all(p.name == "TA-Reborn-test" or p.name.startswith("TA-Reborn-test/") for p in archive))
            self.assertTrue(all(p.uid == p.gid == 0 for p in archive))
            self.assertEqual(archive.extractfile("TA-Reborn-test/assets/reborn/data.txt").read(), b"original project asset")

    def test_extracted_payload_comparison_rejects_missing_or_changed_files(self):
        expected = self.root / "expected"
        self.file("expected/opt/ta-reborn/ta-launcher", b"launcher bytes")
        self.file("expected/DEBIAN/control", b"package metadata")
        actual = self.root / "actual"
        self.file("actual/opt/ta-reborn/ta-launcher", b"launcher bytes")
        pkg.verify_tree(expected, actual, exclude=("DEBIAN",))
        (actual / "opt/ta-reborn/ta-launcher").write_bytes(b"other architecture")
        with self.assertRaisesRegex(ValueError, "differ"):
            pkg.verify_tree(expected, actual, exclude=("DEBIAN",))
        (actual / "opt/ta-reborn/ta-launcher").unlink()
        with self.assertRaisesRegex(ValueError, "differ"):
            pkg.verify_tree(expected, actual, exclude=("DEBIAN",))

    def test_mac_zip_is_exact_signed_app_and_symbols_match_uuid(self):
        app = self.root / "TA Reborn.app"
        launcher = self.file("TA Reborn.app/Contents/MacOS/ta-launcher", b"signed executable")
        zipped = self.root / "app.zip"
        with zipfile.ZipFile(zipped, "w") as archive:
            archive.write(launcher, "TA Reborn.app/Contents/MacOS/ta-launcher")
            archive.writestr("__MACOSX/._TA Reborn.app", b"resource fork")
        pkg.verify_app_zip(app, zipped)
        launcher.write_bytes(b"changed after signing")
        with self.assertRaisesRegex(ValueError, "differ"):
            pkg.verify_app_zip(app, zipped)
        self.assertEqual(pkg.macos_uuids("UUID: 7DAD58ED-9658-3BB3-839E-D553F7E5834B (arm64) /some/app path"),
                         {"arm64": "7dad58ed-9658-3bb3-839e-d553f7e5834b"})
        with self.assertRaisesRegex(ValueError, "Missing"):
            pkg.macos_uuids("no debug symbols")

    def test_macos_signing_orders_nested_code_and_preserves_identity_options(self):
        app = self.root / "TA Reborn.app"
        for identity in ("-", "Developer ID Application: Fixture"):
            with self.subTest(identity=identity), mock.patch.object(pkg, "run") as command:
                pkg.sign_macos_app(app, identity)
            calls = [call.args[0] for call in command.call_args_list]
            self.assertEqual([call[-1] for call in calls[:3]],
                             [app / "Contents/MacOS/ta-reborn", app / "Contents/MacOS/ta-launcher", app])
            for call in calls[:3]:
                self.assertEqual(call[:4], ["codesign", "--force", "--sign", identity])
                self.assertEqual(call[4:-1], [] if identity == "-" else ["--options", "runtime", "--timestamp"])
            self.assertEqual(calls[3], ["codesign", "--verify", "--deep", "--strict", "--verbose=2", app])

    @unittest.skipUnless(pkg.platform.system() == "Darwin", "Native macOS signing is checked before Cargo on both Mac runners")
    def test_native_macos_signs_unsigned_helper_before_main_executable(self):
        pkg.require_tools(["clang", "codesign"])
        app = self.root / "Unsigned fixture.app"
        binaries = app / "Contents/MacOS"
        binaries.mkdir(parents=True)
        info = {"CFBundleIdentifier": "io.github.jared-woodruff.ta-reborn.signing-fixture",
                "CFBundleExecutable": "ta-launcher", "CFBundlePackageType": "APPL", "CFBundleVersion": "1"}
        (app / "Contents/Info.plist").write_bytes(plistlib.dumps(info))
        source = self.file("fixture.c", b"int main(void) { return 0; }\n")
        for name in pkg.PROGRAMS:
            binary = binaries / name
            subprocess.run(["clang", str(source), "-o", str(binary)], check=True)
            # ARM linkers may supply an ad-hoc signature. The regression must also
            # exercise the wholly unsigned Intel input that exposed this bug.
            subprocess.run(["codesign", "--remove-signature", str(binary)], capture_output=True)
            unsigned = subprocess.run(["codesign", "--display", str(binary)], capture_output=True, text=True)
            self.assertNotEqual(unsigned.returncode, 0, unsigned.stdout + unsigned.stderr)
            self.assertIn("code object is not signed at all", unsigned.stderr)
        pkg.sign_macos_app(app, "-")
        for name in pkg.PROGRAMS:
            subprocess.run(["codesign", "--verify", "--strict", str(binaries / name)], check=True)

    def test_resources_have_target_specific_notices_and_never_tools(self):
        repo = self.root / "repo"
        for filename in ("assets/reborn/data.txt", "lang/en.toml", "docs/guide.md", "docs/media/ignored.txt", "docs/LAUNCHER_QA.md", "docs/AIR_REVIEW.md", "docs/RELEASE_QA_MODS.md", "docs/RELEASING.md", "CHANGELOG.md", "LICENSE", "installer/linux/README.txt", "installer/files/mods/README.txt"):
            self.file("repo/" + filename)
        shutil.copy(pkg.REPO / "installer/files/internal_docs.txt", repo / "installer/files/internal_docs.txt")
        calls = []
        def notices(args, **kwargs):
            calls.append(args)
            Path(args[2]).write_text("Fixture notices", encoding="utf-8")
        target = "aarch64-unknown-linux-gnu"
        metadata = {"target": target, "version": "2.1.0-rc.1"}
        with mock.patch.object(pkg, "REPO", repo), mock.patch.object(pkg, "run", notices):
            pkg.stage_resources(self.root / "package", "linux", self.root / "ta-tools", target, metadata)
        package = self.root / "package"
        self.assertTrue((package / "assets/reborn/data.txt").is_file())
        self.assertFalse((package / "docs/media").exists())
        self.assertTrue((package / "docs/guide.md").is_file())
        for internal in ("LAUNCHER_QA.md", "AIR_REVIEW.md", "RELEASE_QA_MODS.md", "RELEASING.md"):
            self.assertFalse((package / "docs" / internal).exists(), internal)
        self.assertFalse((package / "ta-tools").exists())
        self.assertEqual(calls[0][-2:], ["--target", target])
        self.assertEqual(json.loads((package / "build-info.json").read_text()), metadata)

    def test_checksum_artifact_names_match_release_aggregation_contract(self):
        for target, (system, arch, _, _, _) in pkg.TARGETS.items():
            name = f"TA-Reborn-2.1.0-rc.1-{system}-{arch}"
            suffixes = (".tar.gz", "-ubuntu.deb", "-arch.pkg.tar.zst", "-symbols.tar.gz") if system == "linux" else (".zip", ".dmg", "-symbols.tar.gz")
            payloads = [self.file(name + suffix, (target + suffix).encode()) for suffix in suffixes]
            self.file(name + "-SHA256SUMS.txt", "".join(f"{pkg.sha256(p)}  {p.name}\n" for p in payloads).encode())
            expected, sums, _ = release.names("2.1.0-rc.1", target)
            self.assertEqual(set(expected), {p.name for p in payloads})
            self.assertEqual(set(release.validate_group(self.root, "2.1.0-rc.1", target)), set(expected + [sums]))
            payloads[-1].write_bytes(b"changed after checksum")
            with self.assertRaisesRegex(ValueError, "Checksum mismatch"):
                release.validate_group(self.root, "2.1.0-rc.1", target)

    def test_python310_syntax_and_retina_icon(self):
        ast.parse((ROOT / "installer/package_unix.py").read_text(encoding="utf-8"), feature_version=(3, 10))
        png = (ROOT / "installer/macos/emblem-1024.png").read_bytes()
        self.assertEqual(png[:8], b"\x89PNG\r\n\x1a\n")
        self.assertEqual(struct.unpack_from(">II", png, 16), (1024, 1024))


class PortableInstallTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.bash = shutil.which("bash")
        if os.name == "nt":
            cls.bash = "C:/Program Files/Git/bin/bash.exe"
        if not cls.bash or not Path(cls.bash).is_file():
            raise unittest.SkipTest("bash unavailable")

    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="ta-portable-test-")
        self.root = Path(self.temp.name)
        self.addCleanup(self.temp.cleanup)
        self.package = self.root / "portable"
        self.package.mkdir()
        for folder in ("assets", "lang", "docs"):
            (self.package / folder).mkdir()
            (self.package / folder / "file.txt").write_text("fixture", encoding="utf-8")
        for filename in ("ta-launcher", "ta-reborn", "README.txt", "LICENSE.txt", "THIRD-PARTY-NOTICES.txt", "build-info.json", "ta-reborn.png"):
            (self.package / filename).write_text("fixture", encoding="utf-8")
        shutil.copyfile(ROOT / "installer/linux/install.sh", self.package / "install.sh")
        self.data = self.root / "data with spaces"

    def shell_path(self, path):
        value = path.as_posix()
        return "/" + value[0].lower() + value[2:] if os.name == "nt" else value

    def install(self, *args, success=True):
        env = os.environ.copy()
        env["XDG_DATA_HOME"] = self.shell_path(self.data)
        result = subprocess.run([self.bash, self.shell_path(self.package / "install.sh"), *args], env=env, capture_output=True, text=True)
        self.assertEqual(result.returncode == 0, success, result.stdout + result.stderr)
        return result

    def test_shell_syntax_and_lf(self):
        for file in ("installer/build_linux.sh", "installer/build_macos.sh", "installer/linux/install.sh", "installer/linux/PKGBUILD"):
            path = ROOT / file
            self.assertNotIn(b"\r", path.read_bytes())
            subprocess.run([self.bash, "-n", self.shell_path(path)], check=True)

    def test_arch_binary_recipe_copies_both_install_trees(self):
        source = self.root / "verified payload"
        destination = self.root / "arch root"
        destination.mkdir()
        for name in ("opt/ta-reborn/ta-launcher", "usr/bin/ta-launcher"):
            path = source / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(name, encoding="utf-8")
        env = os.environ.copy()
        env.update(TA_REBORN_PAYLOAD=self.shell_path(source), pkgdir=self.shell_path(destination), TA_REBORN_PKGVER="2.1.0_rc.1")
        subprocess.run([self.bash, "-euc", 'source "$1"; package', "test-recipe", self.shell_path(ROOT / "installer/linux/PKGBUILD")], env=env, check=True)
        pkg.verify_tree(source, destination)

    def test_install_upgrade_and_uninstall_preserve_user_files(self):
        if os.name != "nt" and pkg.platform.system() != "Linux":
            self.skipTest("Portable installer is Linux-only")
        self.install()
        installed = self.data / "ta-reborn"
        self.assertTrue((installed / "ta-launcher").is_file())
        entry = self.data / "applications/ta-reborn-launcher.desktop"
        self.assertIn('Exec="', entry.read_text())
        # The game alone, for Steam's "Add a Non-Steam Game".
        game_entry = self.data / "applications/ta-reborn-game.desktop"
        game = game_entry.read_text()
        self.assertIn("Name=TA Reborn (game only)", game)
        self.assertRegex(game, r'Exec="[^"\n]*/ta-reborn/ta-reborn"\n')
        (installed / "mods").mkdir()
        saved = installed / "mods/my-mod.txt"
        saved.write_text("keep my mod", encoding="utf-8")
        (self.package / "assets/file.txt").write_text("upgraded", encoding="utf-8")
        self.install()
        self.assertEqual((installed / "assets/file.txt").read_text(), "upgraded")
        self.install("--uninstall")
        self.assertEqual(saved.read_text(), "keep my mod")
        self.assertFalse((installed / "ta-launcher").exists())
        self.assertFalse(entry.exists())
        self.assertFalse(game_entry.exists())

    def test_unmarked_uninstall_does_not_delete_content(self):
        if os.name != "nt" and pkg.platform.system() != "Linux":
            self.skipTest("Portable installer is Linux-only")
        installed = self.data / "ta-reborn"
        installed.mkdir(parents=True)
        keep = installed / "ta-launcher"
        keep.write_text("not managed", encoding="utf-8")
        self.install("--uninstall", success=False)
        self.assertEqual(keep.read_text(), "not managed")

    @unittest.skipUnless(pkg.platform.system() == "Linux", "Native Linux symlink behavior is checked on the Linux CI jobs")
    def test_nested_destination_symlink_cannot_overwrite_outside_install(self):
        self.install()
        outside = self.root / "outside"
        outside.mkdir()
        keep = outside / "file.txt"
        keep.write_text("keep", encoding="utf-8")
        destination = self.data / "ta-reborn/assets"
        shutil.rmtree(destination)
        destination.symlink_to(outside, target_is_directory=True)
        self.install(success=False)
        self.assertEqual(keep.read_text(), "keep")

    @unittest.skipUnless(pkg.platform.system() == "Linux", "Linux-only installer symlink guard")
    def test_nested_source_symlink_cannot_enter_installed_package(self):
        outside = self.root / "outside.txt"
        outside.write_text("private", encoding="utf-8")
        (self.package / "assets/link.txt").symlink_to(outside)
        self.install(success=False)
        self.assertFalse((self.data / "ta-reborn/assets/link.txt").exists())


if __name__ == "__main__":
    unittest.main()
