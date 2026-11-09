# Release builds and packages

The release pipeline builds the game and launcher natively for six Rust targets:

| Platform | CPU | Target | Release files |
| --- | --- | --- | --- |
| Windows 10/11 | x64 | `x86_64-pc-windows-msvc` | NSIS setup, portable ZIP, PDB symbols |
| Windows 11 | ARM64 | `aarch64-pc-windows-msvc` | NSIS setup, portable ZIP, PDB symbols |
| macOS 13+ | Intel x64 | `x86_64-apple-darwin` | App ZIP, DMG, dSYM symbols |
| macOS 13+ | Apple Silicon | `aarch64-apple-darwin` | App ZIP, DMG, dSYM symbols |
| Ubuntu 22.04+ / Arch Linux | x64 | `x86_64-unknown-linux-gnu` | Portable tarball, DEB, pacman package, debug symbols |
| Ubuntu 22.04+ / Arch Linux ARM | ARM64 | `aarch64-unknown-linux-gnu` | Portable tarball, DEB, pacman package, debug symbols |

Arch Linux itself supports x86_64; the ARM64 package is for **Arch Linux ARM**.
All packages require the player's own original Total Annihilation data. Original game
archives are rejected during staging. The Linux builds use Ubuntu 22.04's glibc baseline;
Vulkan drivers are required. macOS uses Metal. The launcher uses OpenGL.

## Making a release

1. Set `[workspace.package].version` in `Cargo.toml`, update the lockfile and changelog,
   and merge the release changes. Create a matching Git tag, such as `v2.2.0` for `2.2.0`.
2. Run **Actions → Release → Run workflow** against that tag for a rehearsal. This runs
   tests and packaging and produces a `release-files` artifact; a manual run never uploads
   to a GitHub release. **Quality → release_packages** provides the same rehearsal from a
   branch, including before the new release workflow has reached the default branch.
3. Inspect the packages on the target systems, including original-game discovery, audio,
   a real match, launcher skin previews, and installation/removal. Hosted compile and
   synthetic-data tests do not establish graphics-driver or gameplay compatibility.
4. Publish the matching GitHub release. The **Release** workflow builds all six targets,
   verifies their checksums and source revisions, and attaches the complete package set.
   A failure in any target prevents the publish job. A tag/version mismatch fails early.

The workflow uses GitHub's [native x64 and ARM64 hosted runners](https://docs.github.com/en/actions/reference/runners/github-hosted-runners).
Private repositories consume the account's Actions allowance; exhausted allowance or disabled
Actions must be resolved before hosted builds can run. Ordinary pushes and pull requests do
not start the release matrix. The existing Quality workflow remains manual.

Test compilation, test execution and production compilation are separate steps. The `ci`
profile runs the whole test suite at optimization level 1 without LTO or debug symbols,
while retaining release assertion/overflow settings. The production `release` profile still
uses thin LTO and line-table symbols; only those binaries enter packages. Cargo output stays
live, and long steps print elapsed time and compiler CPU/memory summaries every minute.
Compiler timing reports are retained as separate workflow artifacts when available.
Packaging diagnostics still run after test assertion failures so one rehearsal can expose
both issues; failed tests continue to block package upload, assembly and publication.

The uploader refuses to overwrite existing filenames. If upload is interrupted, inspect
the release and its checksums before removing only the incomplete assets and retrying.
Do not replace packages that users have already downloaded; publish a new version instead.

For repositories using [immutable releases](https://docs.github.com/en/code-security/concepts/supply-chain-security/immutable-releases),
use the manual rehearsal first, download and unpack `release-files`, attach those files to
the **draft** release with the uploader below, then publish it. Use a checkout of the same
tag and an authenticated GitHub CLI. The uploader preserves the asset order required by
older launchers; do not upload the whole folder concurrently by hand.

```bash
GITHUB_REPOSITORY=Jared-woodruff/TA-Reborn RELEASE_TAG=v2.2.0 \
  python tools/release/release.py publish --dir dist/publish
```

Replace the example tag and folder with the actual release. GitHub prevents adding assets after an immutable
release is published; the automatic uploader reports that restriction instead of changing it.

## Artifact names and updates

All names start with `TA-Reborn-<version>-<platform>-<arch>`; architectures in filenames
are `x64` and `arm64`:

* Windows: `-setup.exe`, `.zip`, `-symbols.zip`.
* macOS: `.zip`, `.dmg`, `-symbols.tar.gz`.
* Linux: `.tar.gz`, `-ubuntu.deb`, `-arch.pkg.tar.zst`, `-symbols.tar.gz`.

Each target includes `-SHA256SUMS.txt` and `-build.json`. The latter binds the files to
their target, version and full Git commit. The complete release includes a combined
`TA-Reborn-<version>-SHA256SUMS.txt`, including symbols and build records.

`TA-Reborn-<version>-setup.exe` remains an identical copy of the x64 installer for older
launchers. New Windows launchers select the exact version and their own compiled architecture;
ARM64 never falls back to x64. macOS and Linux updates open the release page so users can
update through their chosen package format. Custom update TOML feeds can supply
`setup_windows_x64` / `setup_windows_arm64` and their corresponding `_size` keys.

## Local builds

Use Rust 1.95 or newer. Native builds are the tested packaging path; do not rename an x64
binary to produce an ARM64 package. Packagers check executable headers before staging.

```powershell
# Windows; omit -Target to build the native host into target/release.
./installer/build_installer.ps1 -Target x86_64-pc-windows-msvc -DistDir dist/windows-x64
./installer/build_installer.ps1 -Target aarch64-pc-windows-msvc -DistDir dist/windows-arm64
```

Windows cross-builds also need Visual C++ and Windows SDK ARM64 components and a runnable
host `ta-tools.exe` for notices (`-HostTools`). NSIS 3.13 builds the installer; a small
installer bootstrap installs the architecture-specific Rust binaries. Signing is optional
through the existing `-SignScript` hook.
Windows packages include the required app-local Visual C++ runtime DLLs from Visual
Studio's redistributable directory, with dependency, architecture and minimum-version
checks. `-VCRedistDir` can select an explicit redistributable root for a local build.

```bash
# Ubuntu: build dependencies plus native packaging tools.
sudo apt install build-essential pkg-config libasound2-dev libudev-dev libxkbcommon-dev \
  libxkbcommon-x11-0 libx11-xcb1 \
  libwayland-dev libx11-dev libxcursor-dev libxi-dev libxrandr-dev libgl1-mesa-dev \
  libegl1-mesa-dev libarchive-tools zstd desktop-file-utils dpkg-dev
bash installer/build_linux.sh --target x86_64-unknown-linux-gnu --dist-dir dist/linux-x64
# On ARM64 Ubuntu, use --target aarch64-unknown-linux-gnu.

# macOS, with Xcode Command Line Tools installed:
bash installer/build_macos.sh --target aarch64-apple-darwin --dist-dir dist/macos-arm64
# On an Intel Mac, use --target x86_64-apple-darwin.
```

`--skip-build` reuses `target/<triple>/release`; `--target-dir` selects another Cargo target
root. macOS packages contain both executables in `Contents/MacOS` and shared resources
in `Contents/Resources`. Linux system packages install under `/opt/ta-reborn` with command
wrappers and a desktop entry; the portable archive retains the per-user `install.sh`.

macOS builds are **ad-hoc signed by default**, not Developer ID signed or notarized.
For local distribution signing, set `MACOS_SIGN_IDENTITY` to an installed Developer ID
identity and optionally `MACOS_NOTARY_PROFILE` to an existing notarytool keychain profile.
The hosted workflow does not install private signing credentials. Windows setup is unsigned
unless a signing hook is supplied. Configure signing before claiming a signed public release.

Keep the exact symbols for each published target and version. Never distribute a newer
build's symbols alongside an older binary. The packagers and combined-release tests run
without original game data:

```bash
python -m unittest discover -s tools/release/tests -v
python -m unittest discover -s installer/tests -p 'test_*.py' -v
```

Windows additionally runs `installer/tests/windows_package.ps1` against small synthetic
PE fixtures and the real NSIS compiler. These fixtures verify packaging and architecture
guards; they are not evidence of running the ARM64 game on x64 hardware.
