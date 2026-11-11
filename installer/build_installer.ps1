<#
.SYNOPSIS
    Builds TA Reborn and packages it as a Windows installer (NSIS 3).

.DESCRIPTION
    1. Builds the release binaries:  cargo build --release -p ta_game -p ta_launcher -p ta_tools
    2. Stages the files in dist\stage-windows-<arch>\TA Reborn\ (with licences and the
       third-party notices, which ta-tools writes from Cargo's metadata), with Microsoft's
       DirectX Shader Compiler (dxcompiler.dll, for Direct3D 12) from its pinned release
       (windows_package.ps1), downloaded once into <TargetDir>\dxc\ and checked by SHA-256
    3. Packs the staged folder as a portable archive, and the programs with their symbols
       (for reading crash reports): TA-Reborn-<version>-windows-<arch>.zip and
       TA-Reborn-<version>-windows-<arch>-symbols.zip
    4. Compiles installer\ta_reborn.nsi with makensis.exe (NSIS 3), producing
       TA-Reborn-<version>-windows-<arch>-setup.exe, and writes the SHA-256 sums of
       all three payloads to TA-Reborn-<version>-windows-<arch>-SHA256SUMS.txt.

    This script never installs anything. If NSIS is missing it explains how to get it and
    stops after step 3.

.PARAMETER SkipBuild
    Reuse the existing release binaries instead of running cargo.

.PARAMETER StageOnly
    Stage the files but do not compile the installer.

.PARAMETER TargetDir
    Cargo target directory. Defaults to $env:CARGO_TARGET_DIR, or <repo>\target.

.PARAMETER Target
    Windows Rust target triple: x86_64-pc-windows-msvc or aarch64-pc-windows-msvc.
    An explicit target uses <TargetDir>\<Target>\release. Omit to reuse the native
    <TargetDir>\release layout used by ordinary cargo builds.

.PARAMETER HostTools
    Runnable ta-tools.exe built for rustc's host target. Needed with -SkipBuild when
    cross-packaging and no host build exists. Notices still use the payload's Target.

.PARAMETER VCRedistDir
    Visual Studio VC/Redist/MSVC/<version> folder containing architecture-specific
    release CRT DLLs. Defaults to VCToolsRedistDir or installed Visual Studio discovery.

.PARAMETER DistDir
    Output folder for the staged files and the installer. Defaults to <repo>\dist.

.PARAMETER DxcArchive
    A DirectX Shader Compiler release archive (dxc_<date>.zip) to take dxcompiler.dll from,
    instead of the pinned release (downloaded into <TargetDir>\dxc\ the first time). It
    must still have the SHA-256 -DxcSha256 gives. For the packaging tests, and for trying
    another release before pinning it.

.PARAMETER DxcSha256
    The SHA-256 the DXC archive must have. Defaults to the pinned release's.

.PARAMETER SignScript
    A .cmd, .bat or .exe that signs the file named by its first argument (for example with
    signtool and your code-signing certificate). It signs both programs, the uninstaller and
    the setup. Without it nothing is signed.

.EXAMPLE
    powershell -ExecutionPolicy Bypass -File installer\build_installer.ps1

.EXAMPLE
    .\installer\build_installer.ps1 -SkipBuild -StageOnly
#>
[CmdletBinding()]
param(
    [switch]$SkipBuild,
    [switch]$StageOnly,
    [string]$TargetDir,
    [ValidateSet('x86_64-pc-windows-msvc', 'aarch64-pc-windows-msvc')]
    [string]$Target,
    [string]$HostTools,
    [string]$VCRedistDir,
    [string]$DistDir,
    [string]$DxcArchive,
    [string]$DxcSha256,
    [string]$SignScript
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$InstallerDir = $PSScriptRoot
$RepoRoot = Split-Path -Parent $InstallerDir
. (Join-Path $InstallerDir 'windows_package.ps1')

function Write-Step([string]$Message) {
    Write-Host "==> $Message" -ForegroundColor Cyan
}

function Resolve-RepoPath([string]$Path) {
    if ([System.IO.Path]::IsPathRooted($Path)) { return [IO.Path]::GetFullPath($Path) }
    return [IO.Path]::GetFullPath((Join-Path $RepoRoot $Path))
}

# ---- Paths and version -----------------------------------------------------------------------

if (-not $TargetDir) {
    $TargetDir = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { 'target' }
}
if (-not $DistDir) { $DistDir = 'dist' }
$TargetDir = Resolve-RepoPath $TargetDir
$DistDir = Resolve-RepoPath $DistDir
if (-not (Get-Command rustc -CommandType Application -ErrorAction SilentlyContinue)) { throw 'rustc is required to identify the host tools architecture.' }
$rustInfo = @(& rustc -vV)
if ($LASTEXITCODE -ne 0) { throw 'rustc -vV failed.' }
$HostTarget = ($rustInfo | Where-Object { $_ -match '^host: ' } | Select-Object -First 1) -replace '^host: ', ''
$null = Get-WindowsTarget $HostTarget
$ExplicitTarget = [bool]$Target
if (-not $Target) {
    if ($env:CARGO_BUILD_TARGET) { throw 'CARGO_BUILD_TARGET is set: pass -Target explicitly so the release layout cannot be mistaken for a native build.' }
    $Target = $HostTarget
}
$targetInfo = Get-WindowsTarget $Target
$ReleaseDir = if ($ExplicitTarget) { Join-Path $TargetDir "$Target\release" } else { Join-Path $TargetDir 'release' }
$StageRoot = Join-Path $DistDir "stage-windows-$($targetInfo.Arch)"
$StageDir = Join-Path $StageRoot 'TA Reborn'
[void](Assert-PackagePath $StageDir $DistDir)
foreach ($sourceName in 'assets', 'lang', 'docs') {
    $sourceRoot = (Join-Path $RepoRoot $sourceName).TrimEnd('\')
    if ($StageDir.StartsWith($sourceRoot + '\', [StringComparison]::OrdinalIgnoreCase)) {
        throw "DistDir must not stage packages inside the '$sourceName' input tree."
    }
}
if ($HostTools) { $HostTools = Resolve-RepoPath $HostTools }
if ($VCRedistDir) { $VCRedistDir = Resolve-RepoPath $VCRedistDir }
if ($DxcArchive) { $DxcArchive = Resolve-RepoPath $DxcArchive }
if (-not $DxcSha256) { $DxcSha256 = $script:DxcArchiveSha256 }

# The installer version follows [workspace.package] version in Cargo.toml.
$Version = $null
$section = ''
foreach ($line in Get-Content -LiteralPath (Join-Path $RepoRoot 'Cargo.toml')) {
    if ($line -match '^\s*\[([^\]]+)\]\s*$') { $section = $Matches[1]; continue }
    if ($section -eq 'workspace.package' -and $line -match '^\s*version\s*=\s*"([^"]+)"') {
        $Version = $Matches[1]
        break
    }
}
if (-not $Version) { throw 'Could not read [workspace.package] version from Cargo.toml.' }
Write-Host "TA Reborn $Version" -ForegroundColor Green

# The programs show the commit they are built from; a release should be built from a clean one.
$Commit = ''
if (Get-Command 'git' -ErrorAction SilentlyContinue) {
    $Commit = (& git -C $RepoRoot rev-parse --short=9 HEAD 2>$null)
    $changes = (& git -C $RepoRoot status --porcelain 2>$null)
    if ($changes) {
        Write-Warning "The working tree has uncommitted changes: the build is not exactly commit $Commit."
    }
}

if ($SignScript) {
    $SignScript = Resolve-RepoPath $SignScript
    if (-not (Test-Path -LiteralPath $SignScript -PathType Leaf)) { throw "Sign script $SignScript was not found." }
}

# Zips the files under $Folder with '/' between the names, as the zip format has it (Windows
# PowerShell's Compress-Archive writes '\', which other unzip tools misread). The names start
# with the folder's own name unless -ContentsOnly.
function New-Zip([string]$Folder, [string]$ZipPath, [switch]$ContentsOnly) {
    Add-Type -AssemblyName System.IO.Compression, System.IO.Compression.FileSystem
    [void](Assert-PackagePath $ZipPath $DistDir)
    Assert-PlainPackageTree $Folder
    if (Test-Path -LiteralPath $ZipPath) { Remove-Item -LiteralPath $ZipPath -Force }
    $base = if ($ContentsOnly) { $Folder } else { Split-Path -Parent $Folder }
    $zip = [System.IO.Compression.ZipFile]::Open($ZipPath, [System.IO.Compression.ZipArchiveMode]::Create)
    try {
        foreach ($file in Get-ChildItem -LiteralPath $Folder -Recurse -File) {
            $name = $file.FullName.Substring($base.Length).TrimStart('\').Replace('\', '/')
            [void][System.IO.Compression.ZipFileExtensions]::CreateEntryFromFile($zip, $file.FullName, $name, [System.IO.Compression.CompressionLevel]::Optimal)
        }
    } finally {
        $zip.Dispose()
    }
}

# Signs one file with -SignScript.
function Invoke-Sign([string]$File) {
    Write-Host "    signing $File"
    & $SignScript $File
    if ($LASTEXITCODE -ne 0) { throw "Signing $File failed (exit code $LASTEXITCODE)." }
}

# ---- 1. Build -------------------------------------------------------------------------------

if ($SkipBuild) {
    Write-Step 'Skipping the build (-SkipBuild)'
} else {
    Write-Step 'Building release binaries: cargo build --release -p ta_game -p ta_launcher -p ta_tools'
    if (-not (Get-Command 'cargo' -ErrorAction SilentlyContinue)) {
        throw 'cargo was not found. Install Rust from https://rustup.rs and try again.'
    }
    Push-Location $RepoRoot
    try {
        $buildArgs = @('build', '--release', '--locked', '-p', 'ta_game', '-p', 'ta_launcher', '-p', 'ta_tools', '--target-dir', $TargetDir)
        if ($ExplicitTarget) { $buildArgs += @('--target', $Target) }
        & cargo @buildArgs
        if ($LASTEXITCODE -ne 0) { throw "cargo build failed (exit code $LASTEXITCODE)." }
        if ($Target -ne $HostTarget -and -not $HostTools) {
            & cargo build --release --locked -p ta_tools --target $HostTarget --target-dir $TargetDir
            if ($LASTEXITCODE -ne 0) { throw "Host ta-tools build failed (exit code $LASTEXITCODE)." }
        }
    } finally {
        Pop-Location
    }
}

# ---- 2. Stage -------------------------------------------------------------------------------

Write-Step "Staging files in $StageDir"
# Validate the payload before replacing a previous staging tree.
foreach ($exe in 'ta-reborn.exe', 'ta-launcher.exe') { Assert-PeTarget (Join-Path $ReleaseDir $exe) $Target }
$runtimeFiles = @(Get-MsvcRuntimeFiles @((Join-Path $ReleaseDir 'ta-reborn.exe'), (Join-Path $ReleaseDir 'ta-launcher.exe')) $Target $VCRedistDir)
# Microsoft's DirectX Shader Compiler, which Direct3D 12 compiles the game's shaders with: the
# pinned release's dxcompiler.dll for this architecture (windows_package.ps1), checked too before
# the previous stage is replaced.
$dxcWork = Join-Path $DistDir "dxc-windows-$($targetInfo.Arch)"
Reset-PackageDirectory $dxcWork $DistDir
$dxcSource = if ($DxcArchive) { $DxcArchive } else { Get-DxcArchive (Join-Path $TargetDir 'dxc') }
$dxcDll = Expand-DxcCompiler $dxcSource $DxcSha256 $Target $dxcWork
Reset-PackageDirectory $StageDir $DistDir

foreach ($exe in 'ta-reborn.exe', 'ta-launcher.exe') {
    $source = Join-Path $ReleaseDir $exe
    if (-not (Test-Path -LiteralPath $source -PathType Leaf)) {
        throw "$source was not found. Build the release binaries first (run without -SkipBuild)."
    }
    Copy-Item -LiteralPath $source -Destination $StageDir
}

# App-local release CRT dependencies, copied byte-for-byte from Visual Studio's
# redistributable tree. Do not sign or modify Microsoft's already-signed DLLs.
$runtimeNotice = @(
    'Microsoft Visual C++ Runtime',
    'Copyright (c) Microsoft Corporation. All rights reserved.',
    '',
    'These unmodified Microsoft runtime DLLs are redistributed with TA Reborn under',
    'the Microsoft Software License Terms for the licensed Visual Studio build tools.',
    'They are separate proprietary components, not covered by TA Reborn''s GPL license.',
    'Source: Visual Studio VC/Redist/MSVC/<version>/<architecture>/Microsoft.VC*.CRT.',
    'Redistribution lists and licensing:',
    'https://learn.microsoft.com/en-us/visualstudio/releases/2022/redistribution',
    'https://learn.microsoft.com/en-us/visualstudio/releases/2026/redistribution',
    'https://learn.microsoft.com/en-us/cpp/windows/redistributing-visual-cpp-files',
    '',
    "Target: $Target",
    'Bundled files (name, file version, SHA-256):'
)
foreach ($runtime in $runtimeFiles) {
    $name = Split-Path -Leaf $runtime
    $hash = (Get-FileHash -LiteralPath $runtime -Algorithm SHA256).Hash
    Copy-Item -LiteralPath $runtime -Destination $StageDir
    $copied = Join-Path $StageDir $name
    Assert-PeTarget $copied $Target
    if ((Get-FileHash -LiteralPath $copied -Algorithm SHA256).Hash -ne $hash) { throw "Microsoft runtime changed while copying: $name" }
    $runtimeNotice += "$name | $([Diagnostics.FileVersionInfo]::GetVersionInfo($runtime).FileVersion) | $($hash.ToLowerInvariant())"
}
Set-Content -LiteralPath (Join-Path $StageDir 'MICROSOFT-RUNTIME-NOTICE.txt') -Value $runtimeNotice -Encoding utf8

# The DirectX Shader Compiler, unmodified (Microsoft signs it), with its licence's notice.
$dxcHash = (Get-FileHash -LiteralPath $dxcDll -Algorithm SHA256).Hash.ToLowerInvariant()
Copy-Item -LiteralPath $dxcDll -Destination $StageDir
if ((Get-FileHash -LiteralPath (Join-Path $StageDir 'dxcompiler.dll') -Algorithm SHA256).Hash.ToLowerInvariant() -ne $dxcHash) { throw 'dxcompiler.dll changed while copying.' }
$dxcNotice = @(
    'Microsoft DirectX Shader Compiler (DXC)',
    '',
    "dxcompiler.dll is Microsoft's DirectX Shader Compiler, redistributed unmodified from its",
    "release $($script:DxcRelease) ($(Get-DxcArchiveUrl)).",
    'On Direct3D 12, TA Reborn compiles its shaders with it.',
    "It is distributed under the University of Illinois/NCSA Open Source License (the release's",
    "LICENSE-LLVM.txt), reproduced below. TA Reborn does not ship the release's dxil.dll: from",
    'version 1.8.2505, DXC signs shaders without it.',
    '',
    "Target: $Target",
    'Bundled file (name, file version, SHA-256):',
    "dxcompiler.dll | $([Diagnostics.FileVersionInfo]::GetVersionInfo($dxcDll).FileVersion) | $dxcHash",
    ''
) + @(Get-Content -LiteralPath (Join-Path $dxcWork 'LICENSE-LLVM.txt'))
Set-Content -LiteralPath (Join-Path $StageDir 'DXC-NOTICE.txt') -Value $dxcNotice -Encoding utf8
Remove-PackageDirectory $dxcWork $DistDir

# Engine resources (never original game data).
$assetsSource = Join-Path $RepoRoot 'assets'
$assetsStage = Join-Path $StageDir 'assets'
if (Test-Path -LiteralPath $assetsSource) {
    Assert-PlainPackageTree $assetsSource
    Copy-Item -LiteralPath $assetsSource -Destination $assetsStage -Recurse
} else {
    New-Item -ItemType Directory -Path $assetsStage | Out-Null
}

# Interface languages (lang\<code>.toml, and the template translators start from).
$langSource = Join-Path $RepoRoot 'lang'
if (Test-Path -LiteralPath $langSource) {
    Assert-PlainPackageTree $langSource
    Copy-Item -LiteralPath $langSource -Destination (Join-Path $StageDir 'lang') -Recurse
}

# An empty mods folder with a README.
$modsStage = Join-Path $StageDir 'mods'
New-Item -ItemType Directory -Path $modsStage -Force | Out-Null
Copy-Item -LiteralPath (Join-Path $InstallerDir 'files\mods\README.txt') -Destination $modsStage

# Documentation.
$docsStage = Join-Path $StageDir 'docs'
New-Item -ItemType Directory -Path $docsStage -Force | Out-Null
$docsSource = Join-Path $RepoRoot 'docs'
if (Test-Path -LiteralPath $docsSource) {
    Assert-PlainPackageTree $docsSource
    # (Not the internal notes, nor docs\media, the README's pictures: installer\files\internal_docs.txt.)
    $internal = Get-Content -LiteralPath (Join-Path $InstallerDir 'files\internal_docs.txt') | Where-Object { $_ -and -not $_.StartsWith('#') }
    Get-ChildItem -LiteralPath $docsSource -Force | Where-Object { $name = $_.Name; -not ($internal | Where-Object { $name -like $_ }) } | Copy-Item -Destination $docsStage -Recurse -Force
}
Copy-Item -LiteralPath (Join-Path $InstallerDir 'files\README.txt') -Destination $StageDir
Copy-Item -LiteralPath (Join-Path $RepoRoot 'CHANGELOG.md') -Destination $docsStage

# Licences: our own, and the notices of every library compiled into the programs.
Copy-Item -LiteralPath (Join-Path $RepoRoot 'LICENSE') -Destination (Join-Path $StageDir 'LICENSE.txt')
$tools = if ($HostTools) { $HostTools } elseif ($Target -eq $HostTarget) { Join-Path $ReleaseDir 'ta-tools.exe' } else { Join-Path $TargetDir "$HostTarget\release\ta-tools.exe" }
if (-not (Test-Path -LiteralPath $tools -PathType Leaf)) {
    throw "$tools was not found. Build the host ta-tools binary, pass -HostTools, or run without -SkipBuild."
}
Assert-PeTarget $tools $HostTarget
Push-Location $RepoRoot
try {
    & $tools notices (Join-Path $StageDir 'THIRD-PARTY-NOTICES.txt') --target $Target
    if ($LASTEXITCODE -ne 0) { throw "ta-tools notices failed (exit code $LASTEXITCODE)." }
} finally {
    Pop-Location
}

# TA Reborn must never redistribute the original game's archives.
$gameArchives = @(Get-ChildItem -LiteralPath $StageDir -Recurse -File |
    Where-Object { @('.hpi', '.ufo', '.ccx', '.gp3', '.gpf') -contains $_.Extension.ToLowerInvariant() })
if ($gameArchives.Count -gt 0) {
    $names = ($gameArchives | ForEach-Object { $_.FullName }) -join [Environment]::NewLine
    throw "Refusing to package Total Annihilation archives:$([Environment]::NewLine)$names"
}

Get-ChildItem -LiteralPath $StageDir -Recurse -File | ForEach-Object {
    Write-Host ('    {0}  ({1:N0} KB)' -f $_.FullName.Substring($StageDir.Length + 1), [math]::Ceiling($_.Length / 1KB))
}

if ($SignScript) {
    Write-Step 'Signing the programs'
    foreach ($exe in 'ta-reborn.exe', 'ta-launcher.exe') { Invoke-Sign (Join-Path $StageDir $exe) }
}
foreach ($exe in 'ta-reborn.exe', 'ta-launcher.exe') { Assert-PeTarget (Join-Path $StageDir $exe) $Target }

if ($StageOnly) {
    Write-Step 'Staged only (-StageOnly); no packages were made.'
    exit 0
}

# ---- 3. Portable archive and symbols -------------------------------------------------------

$ArtifactPrefix = "TA-Reborn-$Version-windows-$($targetInfo.Arch)"
$portable = Join-Path $DistDir "$ArtifactPrefix.zip"
Write-Step "Packing the portable archive $portable"
New-Zip $StageDir $portable

# The programs as published and their symbols: what a crash report's addresses are read against.
$symbols = Join-Path $DistDir "$ArtifactPrefix-symbols.zip"
Write-Step "Packing the symbols $symbols"
$symbolStage = Join-Path $DistDir "symbols-windows-$($targetInfo.Arch)"
Reset-PackageDirectory $symbolStage $DistDir
foreach ($pair in @(@('ta-reborn.exe', 'ta_reborn.pdb'), @('ta-launcher.exe', 'ta_launcher.pdb'))) {
    Copy-Item -LiteralPath (Join-Path $StageDir $pair[0]) -Destination $symbolStage
    $pdb = Join-Path $ReleaseDir $pair[1]
    if (Test-Path -LiteralPath $pdb -PathType Leaf) {
        Copy-Item -LiteralPath $pdb -Destination $symbolStage
    } else {
        throw "$pdb was not found: refusing to publish a symbols package without its matching PDB."
    }
}
Set-Content -LiteralPath (Join-Path $symbolStage 'BUILD.txt') -Encoding ascii -Value @(
    "TA Reborn $Version, commit $Commit",
    "Target: $Target",
    'A crash report lists frames as addresses and where the program was loaded ("loaded at").',
    'Address minus that base is the offset into the program (RVA); with the .pdb beside the .exe,',
    'a debugger or llvm-symbolizer turns it into a function and line.'
)
New-Zip $symbolStage $symbols -ContentsOnly
Remove-PackageDirectory $symbolStage $DistDir

# ---- 4. Compile the installer ---------------------------------------------------------------

$makensis = Find-Makensis
if (-not $makensis) {
    Write-Warning 'NSIS (makensis.exe) was not found, so the installer was not built.'
    Write-Host ''
    Write-Host 'The files to install are staged in:'
    Write-Host "    $StageDir"
    Write-Host ''
    Write-Host 'Install NSIS 3, then run this script again (add -SkipBuild to reuse the build):'
    Write-Host '    winget install --id NSIS.NSIS -e'
    Write-Host 'or download it from https://nsis.sourceforge.io/Download'
    exit 1
}

$setup = Join-Path $DistDir "$ArtifactPrefix-setup.exe"
[void](Assert-PackagePath $setup $DistDir)
Write-Step "Compiling the installer with $makensis"
$script = Join-Path $InstallerDir 'ta_reborn.nsi'
# /V3: warnings, errors and the size summary, without echoing every script command.
$nsisArgs = @('/V3', "/DAPP_VERSION=$Version", "/DAPP_ARCH=$($targetInfo.Arch)", "/DSTAGE_DIR=$StageDir", "/DOUTPUT_FILE=$setup")
# The installer signs its uninstaller and itself with the same script (!uninstfinalize, !finalize).
if ($SignScript) { $nsisArgs += "/DSIGN_SCRIPT=$SignScript" }
& $makensis @nsisArgs $script
if ($LASTEXITCODE -ne 0) { throw "makensis failed (exit code $LASTEXITCODE)." }
if (-not (Test-Path -LiteralPath $setup -PathType Leaf)) { throw "makensis did not produce $setup." }

# ---- 5. Checksums of what is published --------------------------------------------------------

$sums = Join-Path $DistDir "$ArtifactPrefix-SHA256SUMS.txt"
[void](Assert-PackagePath $sums $DistDir)
$lines = foreach ($file in $setup, $portable, $symbols) {
    '{0}  {1}' -f (Get-FileHash -LiteralPath $file -Algorithm SHA256).Hash.ToLowerInvariant(), (Split-Path -Leaf $file)
}
Set-Content -LiteralPath $sums -Value $lines -Encoding ascii

Write-Host ''
foreach ($file in $setup, $portable, $symbols, $sums) {
    Write-Host ('Ready: {0}  ({1:N1} MB)' -f $file, ((Get-Item -LiteralPath $file).Length / 1MB)) -ForegroundColor Green
}
