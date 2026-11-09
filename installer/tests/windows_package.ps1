param(
    [string]$Makensis = 'makensis.exe',
    [string]$BuilderShell,
    [string]$OutputDir = (Join-Path $PSScriptRoot '..\..\scratch\windows-packaging-tests')
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$repo = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..\..'))
. (Join-Path $repo 'installer/windows_package.ps1')
$OutputDir = [IO.Path]::GetFullPath($OutputDir)
$case = Join-Path $OutputDir ([Guid]::NewGuid().ToString('N'))
[void][IO.Directory]::CreateDirectory($case)
if ($Makensis -eq 'makensis.exe') {
    $Makensis = Find-Makensis
    if (-not $Makensis) { throw 'NSIS was not found on PATH, in Program Files or in its registry key. Install NSIS 3 or pass -Makensis <path>.' }
} else {
    $Makensis = (Get-Command $Makensis -CommandType Application -ErrorAction Stop | Select-Object -First 1).Source
}
if ($BuilderShell) { $powershell = (Get-Command $BuilderShell -CommandType Application -ErrorAction Stop | Select-Object -First 1).Source }
else {
    $powershell = (Get-Command pwsh.exe -CommandType Application -ErrorAction SilentlyContinue | Select-Object -First 1)
    if ($powershell) { $powershell = $powershell.Source } else { $powershell = (Get-Command powershell.exe | Select-Object -First 1).Source }
}

function Expect([bool]$Condition, [string]$Message) { if (-not $Condition) { throw "FAIL: $Message" } }
function Expect-Failure([scriptblock]$Action, [string]$Contains) {
    $failed = $false
    try { & $Action } catch { $failed = $true; Expect ($_.Exception.Message -like "*$Contains*") "Unexpected failure: $_" }
    Expect $failed "Expected rejection containing '$Contains'"
}
function Invoke-RejectedBuild([string[]]$BuildArguments, [string]$Log) {
    # Windows PowerShell 5.1 wraps redirected native stderr in ErrorRecords. These
    # calls intentionally fail; retain their log and exit code for the assertions.
    $previous = $ErrorActionPreference
    try {
        $ErrorActionPreference = 'Continue'
        & $powershell -NoProfile -ExecutionPolicy Bypass -File (Join-Path $fixture 'installer/build_installer.ps1') @BuildArguments *> $Log
        return $LASTEXITCODE
    } finally { $ErrorActionPreference = $previous }
}
function Write-FixturePe([string]$Path, [int]$Machine, [string[]]$Imports = @('vcruntime140.dll'), [switch]$Delay) {
    [void][IO.Directory]::CreateDirectory((Split-Path -Parent $Path))
    $bytes = [byte[]]::new(2048)
    [BitConverter]::GetBytes([uint16]0x5A4D).CopyTo($bytes, 0)
    [BitConverter]::GetBytes([uint32]64).CopyTo($bytes, 0x3C)
    [BitConverter]::GetBytes([uint32]0x4550).CopyTo($bytes, 64)
    [BitConverter]::GetBytes([uint16]$Machine).CopyTo($bytes, 68)
    [BitConverter]::GetBytes([uint16]1).CopyTo($bytes, 70)
    [BitConverter]::GetBytes([uint16]240).CopyTo($bytes, 84)
    [BitConverter]::GetBytes([uint16]0x20B).CopyTo($bytes, 88)
    [BitConverter]::GetBytes([uint32]512).CopyTo($bytes, 148)
    [BitConverter]::GetBytes([uint32]16).CopyTo($bytes, 196)
    $directory = if ($Delay) { 13 } else { 1 }
    $entrySize = if ($Delay) { 32 } else { 20 }
    [BitConverter]::GetBytes([uint32]0x1000).CopyTo($bytes, 200 + 8 * $directory)
    [BitConverter]::GetBytes([uint32](($Imports.Count + 1) * $entrySize)).CopyTo($bytes, 204 + 8 * $directory)
    [BitConverter]::GetBytes([uint32]0x1000).CopyTo($bytes, 340)
    [BitConverter]::GetBytes([uint32]1536).CopyTo($bytes, 344)
    [BitConverter]::GetBytes([uint32]512).CopyTo($bytes, 348)
    for ($index = 0; $index -lt $Imports.Count; $index++) {
        $entry = 512 + $index * $entrySize
        if ($Delay) { [BitConverter]::GetBytes([uint32]1).CopyTo($bytes, $entry) }
        $nameOffset = if ($Delay) { 4 } else { 12 }
        [BitConverter]::GetBytes([uint32](0x1100 + 128 * $index)).CopyTo($bytes, $entry + $nameOffset)
        [Text.Encoding]::ASCII.GetBytes($Imports[$index]).CopyTo($bytes, 768 + 128 * $index)
    }
    [IO.File]::WriteAllBytes($Path, $bytes)
}

# Reproduce a freshly installed compiler which is absent from PATH. Discovery
# alone uses this copied executable; the package tests below use real NSIS with
# its complete installation and restore the process environment before building.
$discoveryRoot = Join-Path $case 'program-files-fixture'
[void][IO.Directory]::CreateDirectory((Join-Path $discoveryRoot 'NSIS'))
$discoveryCompiler = Join-Path $discoveryRoot 'NSIS/makensis.exe'
Copy-Item -LiteralPath $Makensis -Destination $discoveryCompiler
$discoveryPath = $env:PATH
try {
    $env:PATH = ''
    Expect ((Find-Makensis -InstallFolders @($discoveryRoot)) -eq $discoveryCompiler) 'NSIS discovery requires PATH despite a valid installation folder.'
} finally { $env:PATH = $discoveryPath }

# Test actual packaging with tiny non-runnable payload fixtures. Only the notices stub
# is executed; it records the target argument so cross-build notices cannot use host deps.
$fixture = Join-Path $case 'repo'
$target = Join-Path $fixture 'target'
$dist = Join-Path $fixture 'dist'
$redist = Join-Path $fixture 'VisualStudio/VC/Redist/MSVC/14.99.0'
foreach ($dir in 'installer/files/mods', 'branding', 'assets/reborn', 'lang', 'docs') { [void][IO.Directory]::CreateDirectory((Join-Path $fixture $dir)) }
foreach ($file in 'build_installer.ps1', 'windows_package.ps1', 'ta_reborn.nsi', 'artwork.nsh') {
    Copy-Item -LiteralPath (Join-Path $repo "installer/$file") -Destination (Join-Path $fixture 'installer')
}
foreach ($file in 'ta-reborn.ico', 'installer-header.bmp', 'installer-welcome.bmp') {
    Copy-Item -LiteralPath (Join-Path $repo "branding/$file") -Destination (Join-Path $fixture 'branding')
}
[IO.File]::WriteAllText((Join-Path $fixture 'Cargo.toml'), "[workspace.package]`nversion = `"0.0.0`"`n")
foreach ($file in 'LICENSE', 'CHANGELOG.md', 'installer/files/README.txt', 'installer/files/mods/README.txt', 'assets/reborn/fixture.txt', 'lang/en.toml', 'docs/fixture.md', 'docs/GAME_QA.md', 'docs/RELEASE_QA_MODS.md') {
    [IO.File]::WriteAllText((Join-Path $fixture $file), 'Packaging fixture only; no original game data.')
}
# (The setup reads which documents stay in the repository from this list: the internal notes.)
Copy-Item -LiteralPath (Join-Path $repo 'installer/files/internal_docs.txt') -Destination (Join-Path $fixture 'installer/files')
& git -C $fixture init --quiet
& git -C $fixture add Cargo.toml
& git -C $fixture -c user.name=Fixture -c user.email=fixture@example.invalid commit --quiet -m fixture
if ($LASTEXITCODE -ne 0) { throw 'Could not initialize fixture repository.' }
$hostTriple = ((& rustc -vV) | Where-Object { $_ -match '^host: ' }) -replace '^host: ', ''
$hostInfo = Get-WindowsTarget $hostTriple
$stubSource = Join-Path $case 'notices.rs'
$stub = Join-Path $case 'host-tools.exe'
[IO.File]::WriteAllText($stubSource, 'fn main() { let a: Vec<String> = std::env::args().collect(); assert_eq!(a[1], "notices"); assert_eq!(a[3], "--target"); std::fs::write(&a[2], &a[4]).unwrap(); }')
& rustc --target $hostTriple --crate-name notices_fixture $stubSource -o $stub
if ($LASTEXITCODE -ne 0) { throw 'Could not build the tiny native notices fixture.' }
Assert-PeTarget $stub $hostTriple

$oldPath = $env:PATH
$env:PATH = (Split-Path -Parent $Makensis) + ';' + $oldPath
try {
    foreach ($triple in 'x86_64-pc-windows-msvc', 'aarch64-pc-windows-msvc') {
        $info = Get-WindowsTarget $triple
        $release = Join-Path $target "$triple/release"
        foreach ($exe in 'ta-reborn.exe', 'ta-launcher.exe') { Write-FixturePe (Join-Path $release $exe) $info.Machine }
        $crt = Join-Path $redist "$($info.Arch)/Microsoft.VC143.CRT"
        Write-FixturePe (Join-Path $crt 'vcruntime140.dll') $info.Machine @('kernel32.dll')
        foreach ($pdb in 'ta_reborn.pdb', 'ta_launcher.pdb') { [IO.File]::WriteAllText((Join-Path $release $pdb), 'fixture symbols') }
        $log = Join-Path $case "$($info.Arch)-package.log"
        & $powershell -NoProfile -ExecutionPolicy Bypass -File (Join-Path $fixture 'installer/build_installer.ps1') -SkipBuild -Target $triple -TargetDir $target -DistDir $dist -HostTools $stub -VCRedistDir $redist *> $log
        Expect ($LASTEXITCODE -eq 0) "Packaging $triple failed; see $log"
        $prefix = "TA-Reborn-0.0.0-windows-$($info.Arch)"
        $sums = @(Get-Content -LiteralPath (Join-Path $dist "$prefix-SHA256SUMS.txt"))
        Expect ($sums.Count -eq 3) 'Checksums must cover setup, portable and symbols.'
        foreach ($line in $sums) {
            Expect ($line -match '^([0-9a-f]{64})  (.+)$') 'Invalid SHA-256 manifest line.'
            $digest, $name = $Matches[1], $Matches[2]
            Expect ((Get-FileHash -LiteralPath (Join-Path $dist $name) -Algorithm SHA256).Hash.ToLowerInvariant() -eq $digest) "Hash mismatch: $name"
            Expect ($name.StartsWith($prefix)) 'Architecture names must not collide.'
        }
        $stage = Join-Path $dist "stage-windows-$($info.Arch)/TA Reborn"
        Expect ((Get-Content -LiteralPath (Join-Path $stage 'THIRD-PARTY-NOTICES.txt') -Raw) -eq $triple) 'Notices used host architecture instead of target.'
        Expect (Test-Path -LiteralPath (Join-Path $stage 'docs/fixture.md')) 'A guide in docs must ship.'
        foreach ($internal in 'GAME_QA.md', 'RELEASE_QA_MODS.md') {
            Expect (-not (Test-Path -LiteralPath (Join-Path $stage "docs/$internal"))) "The internal note $internal must stay out of the package (installer/files/internal_docs.txt)."
        }
        Add-Type -AssemblyName System.IO.Compression.FileSystem
        $zip = [IO.Compression.ZipFile]::OpenRead((Join-Path $dist "$prefix.zip"))
        try {
            Expect ($null -ne $zip.GetEntry('TA Reborn/ta-launcher.exe')) 'Portable root folder or path separators are wrong.'
            Expect ($null -ne $zip.GetEntry('TA Reborn/vcruntime140.dll')) 'Required app-local Microsoft runtime is missing.'
            Expect ($null -ne $zip.GetEntry('TA Reborn/MICROSOFT-RUNTIME-NOTICE.txt')) 'Microsoft runtime notice is missing.'
            Expect ((Get-FileHash -LiteralPath (Join-Path $stage 'vcruntime140.dll')).Hash -eq (Get-FileHash -LiteralPath (Join-Path $crt 'vcruntime140.dll')).Hash) 'Runtime DLL was modified.'
            Expect (@($zip.Entries | Where-Object { $_.FullName.Contains('\') }).Count -eq 0) 'ZIP contains Windows path separators.'
        } finally { $zip.Dispose() }
        $preprocessed = @(& $Makensis /PPO /DAPP_VERSION=0.0.0 "/DAPP_ARCH=$($info.Arch)" "/DSTAGE_DIR=$stage" (Join-Path $fixture 'installer/ta_reborn.nsi')) -join "`n"
        Expect ($LASTEXITCODE -eq 0) 'Could not inspect compiled uninstall commands.'
        Expect ($preprocessed -match 'Delete "\$INSTDIR\\vcruntime140\.dll"') 'Uninstaller does not remove its bundled runtime.'
        Expect ($preprocessed -notmatch 'Delete "\$INSTDIR\\msvcp140\.dll"') 'Uninstaller removes a runtime it did not install.'
    }

    # Delay-import discovery includes transitive runtime dependencies. Never accept
    # an x64 (or ARM64EC-only) DLL just because it is in an ARM64-named folder.
    $delayLoader = Join-Path $case 'delay-loader.exe'
    Write-FixturePe $delayLoader 0x8664 @('vcruntime140_1.dll') -Delay
    $extraRuntime = Join-Path $redist 'x64/Microsoft.VC143.CRT/vcruntime140_1.dll'
    Expect-Failure { Get-MsvcRuntimeFiles @($delayLoader) 'x86_64-pc-windows-msvc' $redist } 'is missing'
    Write-FixturePe $extraRuntime 0x8664 @('vcruntime140.dll')
    $runtimeFiles = @(Get-MsvcRuntimeFiles @($delayLoader) 'x86_64-pc-windows-msvc' $redist)
    Expect ($runtimeFiles.Count -eq 2) 'Delay-loaded or transitive runtime dependencies were missed.'
    $bytes = [IO.File]::ReadAllBytes($delayLoader)
    $bytes[90] = 14
    $bytes[91] = 99
    [IO.File]::WriteAllBytes($delayLoader, $bytes)
    Expect-Failure { Get-MsvcRuntimeFiles @($delayLoader) 'x86_64-pc-windows-msvc' $redist } 'predates'
    Expect-Failure { Find-MsvcRedist 'x86_64-pc-windows-msvc' $env:SystemRoot } 'never a Windows system directory'
    $armRuntime = Join-Path $redist 'arm64/Microsoft.VC143.CRT/vcruntime140.dll'
    foreach ($wrongMachine in 0x8664, 0xA641) {
        Write-FixturePe $armRuntime $wrongMachine @('kernel32.dll')
        Expect-Failure { Get-MsvcRuntimeFiles @((Join-Path $target 'aarch64-pc-windows-msvc/release/ta-launcher.exe')) 'aarch64-pc-windows-msvc' $redist } 'Wrong PE architecture'
    }
    $armStage = Join-Path $dist 'stage-windows-arm64/TA Reborn/ta-launcher.exe'
    $before = (Get-FileHash -LiteralPath $armStage).Hash
    $log = Join-Path $case 'wrong-runtime-architecture.log'
    $rejected = Invoke-RejectedBuild @('-SkipBuild', '-Target', 'aarch64-pc-windows-msvc', '-TargetDir', $target, '-DistDir', $dist, '-HostTools', $stub, '-VCRedistDir', $redist) $log
    Expect ($rejected -ne 0) 'Wrong architecture Microsoft runtime was packaged.'
    Expect ((Get-Content -LiteralPath $log -Raw) -match 'Wrong PE architecture') 'Wrong runtime failed for an unrelated reason.'
    Expect ((Get-FileHash -LiteralPath $armStage).Hash -eq $before) 'Invalid runtime replaced the previous stage.'
    Write-FixturePe $armRuntime 0xAA64 @('kernel32.dll')

    # Wrong-machine payloads must fail before disturbing the previous stage.
    $armStage = Join-Path $dist 'stage-windows-arm64/TA Reborn/ta-launcher.exe'
    $before = (Get-FileHash -LiteralPath $armStage).Hash
    Write-FixturePe (Join-Path $target 'aarch64-pc-windows-msvc/release/ta-launcher.exe') 0x8664
    $log = Join-Path $case 'wrong-architecture.log'
    $rejected = Invoke-RejectedBuild @('-SkipBuild', '-Target', 'aarch64-pc-windows-msvc', '-TargetDir', $target, '-DistDir', $dist, '-HostTools', $stub, '-VCRedistDir', $redist) $log
    Expect ($rejected -ne 0) 'Mismatched payload architecture was accepted.'
    Expect ((Get-Content -LiteralPath $log -Raw) -match 'Wrong PE architecture') 'Wrong payload failed for an unrelated reason.'
    Expect ((Get-FileHash -LiteralPath $armStage).Hash -eq $before) 'Invalid payload replaced the previous stage.'

    # Original TA archives are refused even if accidentally added to the input tree.
    $forbidden = Join-Path $fixture 'assets/forbidden.HPI'
    [IO.File]::WriteAllText($forbidden, 'fixture, not original content')
    $log = Join-Path $case 'original-archive-rejected.log'
    $rejected = Invoke-RejectedBuild @('-SkipBuild', '-StageOnly', '-Target', 'x86_64-pc-windows-msvc', '-TargetDir', $target, '-DistDir', $dist, '-HostTools', $stub, '-VCRedistDir', $redist) $log
    Expect ($rejected -ne 0) 'Forbidden original-game archive extension was packaged.'
    Expect ((Get-Content -LiteralPath $log -Raw) -match 'Refusing to package Total Annihilation archives') 'Archive refusal failed for an unrelated reason.'
    Remove-Item -LiteralPath $forbidden

    # Compile a non-elevated ARM64 fixture installer, then verify an x64 host is
    # rejected before installation, registry writes, shortcuts or original-game lookup.
    if ($hostInfo.Arch -eq 'x64') {
        $rejectSetup = Join-Path $case 'arm64-host-check.exe'
        $destination = Join-Path $case 'must-not-install'
        & $Makensis /V2 /DAPP_VERSION=0.0.0 /DAPP_ARCH=arm64 /DTEST_PER_USER "/DSTAGE_DIR=$(Split-Path -Parent $armStage)" "/DOUTPUT_FILE=$rejectSetup" (Join-Path $fixture 'installer/ta_reborn.nsi')
        Expect ($LASTEXITCODE -eq 0) 'ARM64 host-check fixture did not compile.'
        $process = Start-Process -FilePath $rejectSetup -ArgumentList @('/S', '/NODETECT', "/D=$destination") -WindowStyle Hidden -PassThru
        if (-not $process.WaitForExit(30000)) { $process.Kill(); throw 'Architecture rejection timed out.' }
        $process.Refresh()
        Expect ($process.ExitCode -eq 1633) "Wrong host returned $($process.ExitCode), expected 1633."
        Expect (-not (Test-Path -LiteralPath $destination)) 'Wrong architecture installer created an installation folder.'
    }
} finally { $env:PATH = $oldPath }

# Cleanup guard rejects traversal and Windows junctions without touching their targets.
Expect-Failure { Reset-PackageDirectory (Join-Path $case '../outside') $case } 'not inside'
Expect-Failure { Reset-PackageDirectory $case $case } 'not inside'
$protected = Join-Path $case 'protected'
$cleanupRoot = Join-Path $case 'cleanup'
[void][IO.Directory]::CreateDirectory($protected)
[void][IO.Directory]::CreateDirectory($cleanupRoot)
[IO.File]::WriteAllText((Join-Path $protected 'keep.txt'), 'keep')
$link = Join-Path $cleanupRoot 'junction'
New-Item -ItemType Junction -Path $link -Target $protected | Out-Null
try {
    Expect-Failure { Reset-PackageDirectory $cleanupRoot $case } 'linked'
    Expect (Test-Path -LiteralPath (Join-Path $protected 'keep.txt')) 'Linked cleanup deleted outside content.'
} finally { [IO.Directory]::Delete($link) }
Write-Output "PASS: x64/ARM64 packaging, app-local runtime dependency closure/notices/architecture/version/uninstall checks, target notices, three-file checksums, ZIP paths, PE rejection, original-archive rejection, NSIS host rejection and safe cleanup. Fixtures: $case"
# The expected-failure cases deliberately leave a nonzero native LASTEXITCODE.
# Report the suite's asserted outcome explicitly, including when ARM64 skips the
# final x64-only host-rejection check and GitHub's pwsh wrapper reads that variable.
exit 0
