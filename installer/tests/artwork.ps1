param(
    [string]$Makensis = 'makensis.exe',
    [string]$OutputDir = (Join-Path $PSScriptRoot '..\..\scratch\installer-polish\artwork-test')
)
$ErrorActionPreference = 'Stop'
$OutputDir = [IO.Path]::GetFullPath($OutputDir)
[IO.Directory]::CreateDirectory($OutputDir) | Out-Null
$fixture = Join-Path $OutputDir 'checker.bmp'
$setup = Join-Path $OutputDir 'artwork-test.exe'
$result = Join-Path $OutputDir 'artwork-test.log'

# A deterministic 24-bit BMP: alternating black and white pixels, no alpha, padded rows.
# The production macro must average this into gray at every tested destination size.
$width = 256
$height = 192
$stride = [int]([Math]::Ceiling($width * 3 / 4) * 4)
$writer = [IO.BinaryWriter]::new([IO.File]::Create($fixture))
try {
    $writer.Write([byte[]](0x42, 0x4d))
    $writer.Write([uint32](54 + $stride * $height))
    $writer.Write([uint32]0)
    $writer.Write([uint32]54)
    $writer.Write([uint32]40)
    $writer.Write([int32]$width)
    $writer.Write([int32]$height)
    $writer.Write([uint16]1)
    $writer.Write([uint16]24)
    $writer.Write([uint32]0)
    $writer.Write([uint32]($stride * $height))
    $writer.Write([int32]2835)
    $writer.Write([int32]2835)
    $writer.Write([uint32]0)
    $writer.Write([uint32]0)
    for ($y = 0; $y -lt $height; $y++) {
        $row = [byte[]]::new($stride)
        for ($x = 0; $x -lt $width; $x++) {
            $pixel = [byte](255 * (($x + $y) % 2))
            $row[$x * 3] = $pixel
            $row[$x * 3 + 1] = $pixel
            $row[$x * 3 + 2] = $pixel
        }
        $writer.Write($row)
    }
} finally {
    $writer.Dispose()
}

& $Makensis /V2 "/DOUTPUT_FILE=$setup" "/DFIXTURE_FILE=$fixture" "/DRESULT_FILE=$result" (Join-Path $PSScriptRoot 'artwork.nsi')
if ($LASTEXITCODE -ne 0) { throw "NSIS test compilation failed: $LASTEXITCODE" }
if (Test-Path -LiteralPath $result) { Remove-Item -LiteralPath $result }
# No elevation, registry writes, installation or visible nsDialogs page.
$process = Start-Process -FilePath $setup -WindowStyle Hidden -PassThru
if (-not $process.WaitForExit(30000)) {
    $process.Kill()
    throw 'Artwork regression timed out.'
}
$process.Refresh()
if (-not (Test-Path -LiteralPath $result)) { throw 'Artwork regression produced no result.' }
$report = Get-Content -LiteralPath $result -Raw
Write-Output $report
if ($process.ExitCode -ne 0 -or $report -notmatch '(?m)^PASS:') {
    throw "Artwork regression failed: exit $($process.ExitCode)"
}
