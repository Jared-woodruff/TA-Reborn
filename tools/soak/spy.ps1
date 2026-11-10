# A soak test of the game's memory (Windows): `--demo spy` (apps/ta_game/src/app/spy.rs) plays a
# skirmish both of whose sides the computer plays, fast-forwarded, then keeps radar planes of the
# player's over the enemy's base; this samples the game's own memory (private bytes) every two
# seconds and fails when, after the warm-up, it keeps climbing faster than -MaxGrowthMb a minute
# of battle. The settings live in a folder of their own (a small window, the sound off), never
# the player's.
#
#   pwsh -File tools/soak/spy.ps1 [-Map "Coast To Coast"] [-Minutes 10] [-Extra "TA_SPY_GOD=1;TA_SPY_RIDE=1"]
#        [-Exe target/release/ta-reborn.exe] [-MaxGrowthMb 40]
param(
    [string]$Map = "Coast To Coast",
    [int]$Minutes = 10,
    [string]$Extra = "TA_SPY_GOD=1",
    [string]$Exe = "",
    [double]$MaxGrowthMb = 40
)
$ErrorActionPreference = "Stop"
$root = (Resolve-Path "$PSScriptRoot\..\..").Path
if (-not $Exe) { $Exe = "$root\target\release\ta-reborn.exe" }
$work = Join-Path ([System.IO.Path]::GetTempPath()) "ta-soak-$PID"
New-Item -ItemType Directory -Force "$work\appdata\TA Reborn" | Out-Null
@"
[graphics]
preset = "High"
width = 1280
height = 720
window_mode = "Windowed"
vsync = false

[audio]
master = 0.0

[gameplay]
autosave = false
"@ | Set-Content -Encoding utf8 "$work\appdata\TA Reborn\config.toml"
$env:APPDATA = "$work\appdata"
$env:TA_CAPTURE_FPS = "30"
$env:TA_TIPS = "0"
foreach ($kv in $Extra.Split(';', [System.StringSplitOptions]::RemoveEmptyEntries)) {
    $k, $v = $kv.Split('=', 2)
    Set-Item -Path "env:$k" -Value $v
}
# (The battle's minutes on the capture clock: 30 frames a second.)
$frames = 30 * 60 * $Minutes
$log = "$work\game.log"
$argv = @('--demo', 'spy', '--map', "`"$Map`"", '--screenshot', "`"$work\last.png`"", '--frames', "$frames")
$p = Start-Process -FilePath $Exe -ArgumentList $argv -PassThru -RedirectStandardError $log -RedirectStandardOutput "$work\out.log"
$samples = @()
while (-not $p.HasExited) {
    try { $samples += (Get-Process -Id $p.Id -ErrorAction Stop).PrivateMemorySize64 / 1MB } catch {}
    Start-Sleep -Milliseconds 2000
}
# The battle's memory as the game logged it every ten seconds of battle (`spy: ...`).
$points = foreach ($line in Get-Content $log) {
    if ($line -match 'spy: (\d+) s of battle.*private (\d+) MB') { [pscustomobject]@{ s = [double]$Matches[1]; mb = [double]$Matches[2] } }
}
if ($p.ExitCode -ne 0 -or $points.Count -lt 12) {
    Write-Host "soak: the game exited with $($p.ExitCode) after $($points.Count) memory lines (log: $log)"
    exit 1
}
# (After the first minute of the soak; a least-squares line through the rest.)
$rest = $points | Where-Object { $_.s -ge $points[0].s + 60 }
$n = $rest.Count
$mx = ($rest | Measure-Object -Property s -Average).Average
$my = ($rest | Measure-Object -Property mb -Average).Average
$num = 0.0; $den = 0.0
foreach ($q in $rest) { $num += ($q.s - $mx) * ($q.mb - $my); $den += ($q.s - $mx) * ($q.s - $mx) }
$perMinute = if ($den -gt 0) { $num / $den * 60 } else { 0 }
$peak = ($samples | Measure-Object -Maximum).Maximum
Write-Host ("soak on {0}: {1} lines over {2:N0} s of battle, {3:N0} MB to {4:N0} MB, the trend {5:+0.0;-0.0} MB a minute (allowed {6}), the most {7:N0} MB" -f $Map, $n, ($rest[-1].s - $rest[0].s), $rest[0].mb, $rest[-1].mb, $perMinute, $MaxGrowthMb, $peak)
if ($perMinute -gt $MaxGrowthMb) {
    Write-Host "soak: the memory keeps climbing (log: $log)"
    exit 1
}
Remove-Item -Recurse -Force $work
exit 0
