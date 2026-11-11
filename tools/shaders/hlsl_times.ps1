# Times Microsoft's shader compilers on the HLSL that wgpu's DirectX 12 backend would give them, as
# `cargo test --release -p ta_render --lib dump_hlsl -- --ignored` writes it (TA_HLSL_OUT):
# fxc (the compiler every Windows computer has; wgpu falls back to it without dxcompiler.dll) and
# dxc, with the flags wgpu passes, each compile stopped after -TimeoutSec. The compilers are the
# Windows SDK's.
#
#   pwsh -File tools/shaders/hlsl_times.ps1 -Dir scratch/hlsl/base [-Filter "terrain.*"] [-TimeoutSec 120] [-Compiler fxc|dxc|both]
param(
    [Parameter(Mandatory = $true)][string]$Dir,
    [string]$Filter = "*",
    [int]$TimeoutSec = 120,
    [ValidateSet("fxc", "dxc", "both")][string]$Compiler = "both",
    [string]$Sdk = ""
)
if (-not $Sdk) {
    $Sdk = Get-ChildItem "C:\Program Files (x86)\Windows Kits\10\bin" -Directory -Filter "10.*" | Sort-Object Name -Descending |
        Where-Object { Test-Path (Join-Path $_.FullName "x64\dxc.exe") } | Select-Object -First 1 | ForEach-Object { Join-Path $_.FullName "x64" }
}
$fxc = Join-Path $Sdk "fxc.exe"
$dxc = Join-Path $Sdk "dxc.exe"

function Measure-Compile([string]$exe, [string[]]$arguments) {
    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    $p = Start-Process -FilePath $exe -ArgumentList $arguments -PassThru -NoNewWindow -RedirectStandardOutput "$env:TEMP\hlsl_times.out" -RedirectStandardError "$env:TEMP\hlsl_times.err"
    if (-not $p.WaitForExit($TimeoutSec * 1000)) {
        Stop-Process -Id $p.Id -Force
        return ">{0}" -f $TimeoutSec
    }
    $sw.Stop()
    if ($p.ExitCode -ne 0) { return "error" }
    return "{0:N1}" -f $sw.Elapsed.TotalSeconds
}

$rows = foreach ($file in Get-ChildItem (Join-Path $Dir "$Filter.hlsl") | Sort-Object Name) {
    # <shader>.<entry>.<vs|ps|cs>.<bounded|plain>.hlsl
    $parts = $file.BaseName.Split('.')
    $entry, $stage = $parts[1], $parts[2]
    $row = [ordered]@{ shader = $parts[0]; entry = $entry; loops = $parts[3] }
    if ($Compiler -ne "dxc") {
        $row.fxc = Measure-Compile $fxc @('/nologo', '/Ges', '/T', "$($stage)_5_1", '/E', $entry, '/Fo', 'NUL', "`"$($file.FullName)`"")
    }
    if ($Compiler -ne "fxc") {
        $row.dxc = Measure-Compile $dxc @('-E', $entry, '-T', "$($stage)_6_0", '-HV', '2018', '-no-warnings', '-Ges', '-Fo', 'NUL', "`"$($file.FullName)`"")
    }
    [pscustomobject]$row
}
$rows | Format-Table -AutoSize | Out-String -Width 200
