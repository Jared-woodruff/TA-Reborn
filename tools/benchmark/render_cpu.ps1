<#
.SYNOPSIS
Compare renderer CPU preparation with the pre-optimisation implementation.
.DESCRIPTION
First prepare dependencies from the checkout: cargo test -p ta_render --lib --locked
Then run this script after other builds and benchmarks finish. It compiles one small
optimised harness, checks exact output equivalence, and times baseline/current operations
in alternating order. It does not measure whole-frame FPS or GPU time.
The baseline lights source is read from Git; no original game assets are needed.
.EXAMPLE
powershell -ExecutionPolicy Bypass -File tools/benchmark/render_cpu.ps1
.EXAMPLE
powershell -ExecutionPolicy Bypass -File tools/benchmark/render_cpu.ps1 -Quick -OutputDir "C:\Temp\renderer probe"
#>
[CmdletBinding()]
param(
    [string]$OutputDir,
    # Override for dependencies built with a custom target/profile.
    [string]$DepsDir,
    [ValidatePattern('^[0-9a-fA-F]{7,40}$')]
    [string]$Baseline = '7a4c455',
    # Same equivalence checks, fewer timing iterations; for a quick smoke check.
    [switch]$Quick
)

$ErrorActionPreference = 'Stop'
$taskRepo = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../..'))
$taskUtf8 = New-Object System.Text.UTF8Encoding($false)

function ConvertTo-RustString([string]$Value) {
    '"' + $Value.Replace('\', '\\').Replace('"', '\"').Replace("`r", '\r').Replace("`n", '\n') + '"'
}

Write-Host 'Dependency preparation: cargo test -p ta_render --lib --locked'
Write-Host 'Run timings in a quiet session, after other builds and benchmarks finish.'
if (-not $DepsDir) {
    $taskMetadata = & cargo metadata --manifest-path (Join-Path $taskRepo 'Cargo.toml') --no-deps --format-version 1 --locked --offline
    if ($LASTEXITCODE -ne 0) { throw 'Cannot locate Cargo output. Run the preparation command first, or supply -DepsDir.' }
    $DepsDir = Join-Path (($taskMetadata -join "`n" | ConvertFrom-Json).target_directory) 'debug/deps'
}
$DepsDir = [IO.Path]::GetFullPath($DepsDir)
$taskGlam = Get-ChildItem -LiteralPath $DepsDir -Filter 'libglam-*.rlib' | Sort-Object LastWriteTime -Descending | Select-Object -First 1
$taskBytemuck = Get-ChildItem -LiteralPath $DepsDir -Filter 'libbytemuck-*.rlib' | Sort-Object LastWriteTime -Descending | Select-Object -First 1
if (-not $taskGlam -or -not $taskBytemuck) { throw "Compiled dependencies missing from $DepsDir. Run the preparation command first." }

if (-not $OutputDir) {
    $OutputDir = Join-Path ([IO.Path]::GetTempPath()) ('ta-reborn-render-cpu-' + [Guid]::NewGuid().ToString('N'))
}
$OutputDir = [IO.Path]::GetFullPath($OutputDir)
[IO.Directory]::CreateDirectory($OutputDir) | Out-Null
$taskBaselinePath = Join-Path $OutputDir 'baseline-lights.rs'
$taskBaselineSource = & git -C $taskRepo show "${Baseline}:crates/ta_render/src/lights.rs"
if ($LASTEXITCODE -ne 0) { throw "Cannot read baseline $Baseline. Fetch the repository history containing that commit." }
[IO.File]::WriteAllText($taskBaselinePath, ($taskBaselineSource -join "`n"), $taskUtf8)

$taskGenerated = Join-Path $OutputDir 'render_cpu.generated.rs'
$taskModules = @(
    '#![allow(dead_code)]'
    ('#[path = {0}] mod old_lights;' -f (ConvertTo-RustString $taskBaselinePath))
    ('#[path = {0}] mod new_lights;' -f (ConvertTo-RustString (Join-Path $taskRepo 'crates/ta_render/src/lights.rs')))
    ('#[path = {0}] mod batching;' -f (ConvertTo-RustString (Join-Path $taskRepo 'crates/ta_render/src/model/batching.rs')))
)
$taskHarness = [IO.File]::ReadAllText((Join-Path $PSScriptRoot 'render_cpu.rs'))
[IO.File]::WriteAllText($taskGenerated, (($taskModules -join "`n") + "`n" + $taskHarness), $taskUtf8)
$taskExe = Join-Path $OutputDir 'render_cpu.exe'
$taskArgs = @('--edition=2024', '-O', '--crate-name', 'ta_render_cpu_bench', $taskGenerated,
    '-L', "dependency=$DepsDir", '--extern', "glam=$($taskGlam.FullName)",
    '--extern', "bytemuck=$($taskBytemuck.FullName)", '-o', $taskExe)
& rustc @taskArgs
if ($LASTEXITCODE -ne 0) { throw 'Renderer CPU benchmark compilation failed. Rebuild dependencies with the current Rust toolchain.' }

$taskRunArgs = @()
if ($Quick) { $taskRunArgs += '--quick' }
$taskReport = Join-Path $OutputDir 'render_cpu.txt'
& $taskExe @taskRunArgs | Tee-Object -FilePath $taskReport
if ($LASTEXITCODE -ne 0) { throw 'Renderer CPU benchmark failed.' }
Write-Host "Report and generated harness: $OutputDir"
