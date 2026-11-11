# Shared Windows packaging checks; dot-source from the builder and fixture tests.
Set-StrictMode -Version Latest
$script:MsvcRuntimeNames = @('vcruntime140.dll', 'vcruntime140_1.dll', 'vcruntime140_threads.dll',
    'msvcp140.dll', 'msvcp140_1.dll', 'msvcp140_2.dll', 'msvcp140_atomic_wait.dll',
    'msvcp140_codecvt_ids.dll', 'concrt140.dll', 'vccorlib140.dll')

# Microsoft's DirectX Shader Compiler (DXC). Its dxcompiler.dll ships beside the programs, and
# Direct3D 12 compiles the game's shaders with it (crates/ta_render/src/gpu.rs), several times
# faster than with FXC, the compiler every Windows has. The release is pinned by its archive's
# SHA-256 (the digest GitHub lists for the asset). From 1.8.2505 DXC signs shaders itself, so the
# release's dxil.dll, which has a licence of its own, is not shipped. To update: change these
# values, run installer/tests/windows_package.ps1, and play a battle with WGPU_BACKEND=dx12.
$script:DxcRelease = 'v1.9.2609'
$script:DxcArchiveName = 'dxc_2026_09_29.zip'
$script:DxcArchiveSha256 = 'ad31b1fc8443175d204f77a611fdb3ef2ec42759bdc2f1167368de24a4a7e7f1'
$script:DxcSelfSigning = [version]'1.8.2505'

function Get-DxcArchiveUrl {
    return "https://github.com/microsoft/DirectXShaderCompiler/releases/download/$($script:DxcRelease)/$($script:DxcArchiveName)"
}

function Assert-FileSha256([string]$Path, [string]$Sha256) {
    $actual = (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($actual -ne $Sha256.ToLowerInvariant()) { throw "$Path has SHA-256 $actual, expected $($Sha256.ToLowerInvariant()): refusing it." }
}

# The pinned DXC release archive, kept in $CacheDir: downloaded there the first time, checked
# before it is kept.
function Get-DxcArchive([string]$CacheDir) {
    [void][IO.Directory]::CreateDirectory($CacheDir)
    $archive = Join-Path $CacheDir $script:DxcArchiveName
    if (-not (Test-Path -LiteralPath $archive -PathType Leaf)) {
        $partial = "$archive.part"
        $url = Get-DxcArchiveUrl
        Write-Host "    downloading $url"
        # (Windows PowerShell 5.1 offers TLS 1.2 only when asked, and its progress bar slows the
        # download down.)
        [Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12
        $progress = $ProgressPreference
        try {
            $ProgressPreference = 'SilentlyContinue'
            Invoke-WebRequest -Uri $url -OutFile $partial -UseBasicParsing
        } finally { $ProgressPreference = $progress }
        try { Assert-FileSha256 $partial $script:DxcArchiveSha256 }
        catch { Remove-Item -LiteralPath $partial -Force -ErrorAction SilentlyContinue; throw }
        Move-Item -LiteralPath $partial -Destination $archive -Force
    }
    return $archive
}

# Extracts $Target's dxcompiler.dll and the release's LLVM licence (LICENSE-LLVM.txt) from a DXC
# release archive, which must have SHA-256 $Sha256, into $Destination; returns the DLL's path.
function Expand-DxcCompiler([string]$Archive, [string]$Sha256, [string]$Target, [string]$Destination) {
    Assert-FileSha256 $Archive $Sha256
    $arch = (Get-WindowsTarget $Target).Arch
    Add-Type -AssemblyName System.IO.Compression, System.IO.Compression.FileSystem
    $zip = [IO.Compression.ZipFile]::OpenRead($Archive)
    try {
        # (Microsoft's archives name their files with '\'.)
        foreach ($wanted in @(@("bin/$arch/dxcompiler.dll", 'dxcompiler.dll'), @('LICENSE-LLVM.txt', 'LICENSE-LLVM.txt'))) {
            $entry = $zip.Entries | Where-Object { $_.FullName.Replace('\', '/') -eq $wanted[0] } | Select-Object -First 1
            if (-not $entry) { throw "The DXC archive $Archive has no $($wanted[0])." }
            [IO.Compression.ZipFileExtensions]::ExtractToFile($entry, (Join-Path $Destination $wanted[1]), $true)
        }
    } finally { $zip.Dispose() }
    $dll = Join-Path $Destination 'dxcompiler.dll'
    Assert-PeTarget $dll $Target
    return $dll
}

# Chocolatey can install NSIS without updating the current process's PATH. Keep
# packaging, its fixtures and CI preflight on the same standard-folder/registry lookup.
function Find-Makensis([string[]]$InstallFolders = @(${env:ProgramFiles(x86)}, $env:ProgramFiles)) {
    $onPath = Get-Command 'makensis.exe' -CommandType Application -ErrorAction SilentlyContinue |
        Select-Object -First 1
    if ($onPath) { return $onPath.Source }
    $candidates = [Collections.Generic.List[string]]::new()
    foreach ($folder in $InstallFolders) {
        if ($folder) { $candidates.Add((Join-Path $folder 'NSIS\makensis.exe')) }
    }
    foreach ($key in 'HKLM:\SOFTWARE\WOW6432Node\NSIS', 'HKLM:\SOFTWARE\NSIS') {
        $item = Get-Item -LiteralPath $key -ErrorAction SilentlyContinue
        if ($item) {
            $nsisDir = $item.GetValue('')
            if ($nsisDir) { $candidates.Add((Join-Path $nsisDir 'makensis.exe')) }
        }
    }
    foreach ($candidate in $candidates) {
        if (Test-Path -LiteralPath $candidate -PathType Leaf) { return $candidate }
    }
    return $null
}

function Get-WindowsTarget([string]$Triple) {
    switch ($Triple) {
        'x86_64-pc-windows-msvc' { return [pscustomobject]@{ Triple = $Triple; Arch = 'x64'; Machine = 0x8664 } }
        'aarch64-pc-windows-msvc' { return [pscustomobject]@{ Triple = $Triple; Arch = 'arm64'; Machine = 0xAA64 } }
        default { throw "Unsupported Windows release target '$Triple'. Use x86_64-pc-windows-msvc or aarch64-pc-windows-msvc." }
    }
}

function Get-PeMachine([string]$Path) {
    $reader = [IO.BinaryReader]::new([IO.File]::OpenRead($Path))
    try {
        if ($reader.BaseStream.Length -lt 64 -or $reader.ReadUInt16() -ne 0x5A4D) { throw "$Path is not a Windows PE executable." }
        $reader.BaseStream.Position = 0x3C
        $offset = $reader.ReadUInt32()
        if ($offset -lt 64 -or $offset -gt 16MB -or $offset + 26 -gt $reader.BaseStream.Length) { throw "$Path has an invalid PE header offset." }
        $reader.BaseStream.Position = $offset
        if ($reader.ReadUInt32() -ne 0x4550) { throw "$Path has no PE signature." }
        $machine = $reader.ReadUInt16()
        $reader.BaseStream.Position = $offset + 24
        if ($reader.ReadUInt16() -ne 0x20B) { throw "$Path is not a 64-bit PE32+ executable." }
        return $machine
    } finally { $reader.Dispose() }
}

function Assert-PeTarget([string]$Path, [string]$Target) {
    $expected = Get-WindowsTarget $Target
    $actual = Get-PeMachine $Path
    if ($actual -ne $expected.Machine) {
        throw ('Wrong PE architecture in {0}: machine 0x{1:X4}, expected {2} (0x{3:X4}).' -f $Path, $actual, $Target, $expected.Machine)
    }
}

function Resolve-PeOffset([long]$Rva, [long]$Length, $Sections, [long]$HeaderSize, [long]$FileSize) {
    if ($Rva -ge 0 -and $Rva + $Length -le $HeaderSize -and $Rva + $Length -le $FileSize) { return $Rva }
    foreach ($section in $Sections) {
        $offset = $Rva - $section.Address
        if ($offset -ge 0 -and $offset + $Length -le $section.Size -and $section.Offset + $offset + $Length -le $FileSize) {
            return $section.Offset + $offset
        }
    }
    throw "Invalid PE import address: $Rva"
}

# Read both ordinary and delay-loaded imports without executing the target image or
# relying on a host-specific dumpbin. Bounds apply to every header, table and name.
function Get-PeImports([string]$Path) {
    $null = Get-PeMachine $Path
    $reader = [IO.BinaryReader]::new([IO.File]::OpenRead($Path))
    try {
        $reader.BaseStream.Position = 0x3C
        $pe = [long]$reader.ReadUInt32()
        $reader.BaseStream.Position = $pe + 6
        $count = $reader.ReadUInt16()
        $reader.BaseStream.Position = $pe + 20
        $optionalSize = $reader.ReadUInt16()
        if ($count -gt 96 -or $optionalSize -lt 112 -or $optionalSize -gt 4096 -or $pe + 24 + $optionalSize + 40 * $count -gt $reader.BaseStream.Length) { throw "Invalid PE section table: $Path" }
        $optional = $pe + 24
        $reader.BaseStream.Position = $optional + 60
        $headerSize = $reader.ReadUInt32()
        $reader.BaseStream.Position = $optional + 108
        $directories = $reader.ReadUInt32()
        $sections = @()
        for ($index = 0; $index -lt $count; $index++) {
            $reader.BaseStream.Position = $optional + $optionalSize + $index * 40 + 12
            $sections += [pscustomobject]@{ Address = [long]$reader.ReadUInt32(); Size = [long]$reader.ReadUInt32(); Offset = [long]$reader.ReadUInt32() }
        }
        $imports = [Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
        foreach ($directory in 1, 13) {
            if ($directories -le $directory) { continue }
            if (112 + 8 * ($directory + 1) -gt $optionalSize) { throw "Truncated PE import directory: $Path" }
            $reader.BaseStream.Position = $optional + 112 + 8 * $directory
            $rva = [long]$reader.ReadUInt32()
            $size = [long]$reader.ReadUInt32()
            if ($rva -eq 0 -and $size -eq 0) { continue }
            $entrySize = if ($directory -eq 1) { 20 } else { 32 }
            if ($rva -eq 0 -or $size -lt $entrySize -or $size -gt 1MB) { throw "Invalid PE import directory: $Path" }
            $terminated = $false
            for ($index = 0; $index -lt 1024 -and ($index + 1) * $entrySize -le $size; $index++) {
                $reader.BaseStream.Position = Resolve-PeOffset ($rva + $index * $entrySize) $entrySize $sections $headerSize $reader.BaseStream.Length
                $entry = $reader.ReadBytes($entrySize)
                if (@($entry | Where-Object { $_ -ne 0 }).Count -eq 0) { $terminated = $true; break }
                if ($directory -eq 13 -and [BitConverter]::ToUInt32($entry, 0) -ne 1) { throw "Unsupported PE delay-import addresses: $Path" }
                $nameOffset = if ($directory -eq 1) { 12 } else { 4 }
                $nameRva = [BitConverter]::ToUInt32($entry, $nameOffset)
                $name = [Text.StringBuilder]::new()
                $ended = $false
                for ($letter = 0; $letter -lt 256; $letter++) {
                    $reader.BaseStream.Position = Resolve-PeOffset ([long]$nameRva + $letter) 1 $sections $headerSize $reader.BaseStream.Length
                    $byte = $reader.ReadByte()
                    if ($byte -eq 0) { $ended = $true; break }
                    [void]$name.Append([char]$byte)
                }
                if (-not $ended -or $name.ToString() -notmatch '^[a-zA-Z0-9_.-]+\.dll$') { throw "Invalid PE import name: $Path" }
                [void]$imports.Add($name.ToString().ToLowerInvariant())
            }
            if (-not $terminated) { throw "Unterminated PE import table: $Path" }
        }
        return @($imports | Sort-Object)
    } finally { $reader.Dispose() }
}

function Find-MsvcRedist([string]$Target, [string]$Override) {
    $arch = (Get-WindowsTarget $Target).Arch
    $roots = [Collections.Generic.List[string]]::new()
    if ($Override) { $roots.Add([IO.Path]::GetFullPath($Override)) }
    else {
        if ($env:VCToolsRedistDir) { $roots.Add($env:VCToolsRedistDir) }
        $vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio/Installer/vswhere.exe'
        if (Test-Path -LiteralPath $vswhere -PathType Leaf) {
            foreach ($installation in @(& $vswhere -products '*' -property installationPath)) {
                $redist = Join-Path $installation 'VC/Redist/MSVC'
                if (Test-Path -LiteralPath $redist) {
                    foreach ($version in @(Get-ChildItem -LiteralPath $redist -Directory | Where-Object { $_.Name -match '^\d+\.\d+\.\d+$' } | Sort-Object { [version]$_.Name } -Descending)) {
                        $roots.Add($version.FullName)
                    }
                }
            }
        }
    }
    $choices = @()
    foreach ($root in $roots) {
        $root = [IO.Path]::GetFullPath($root).TrimEnd('\', '/')
        if ($root -notmatch '(?i)[\\/]VC[\\/]Redist[\\/]MSVC[\\/](?:\d+\.\d+\.\d+|v\d+)$') {
            if ($Override) { throw 'VCRedistDir must be a Visual Studio VC/Redist/MSVC/<version> directory, never a Windows system directory.' }
            continue
        }
        $archRoot = Join-Path $root $arch
        if (-not (Test-Path -LiteralPath $archRoot -PathType Container)) { continue }
        $crt = @(Get-ChildItem -LiteralPath $archRoot -Directory | Where-Object { $_.Name -match '^Microsoft\.VC\d+\.CRT$' } | Sort-Object Name -Descending)
        if ($crt.Count -eq 1) {
            Assert-PlainPackageTree $crt[0].FullName
            $runtime = Join-Path $crt[0].FullName 'vcruntime140.dll'
            if (Test-Path -LiteralPath $runtime -PathType Leaf) {
                $info = [Diagnostics.FileVersionInfo]::GetVersionInfo($runtime)
                $choices += [pscustomobject]@{ Path = $crt[0].FullName; Version = [version]::new($info.FileMajorPart, $info.FileMinorPart, $info.FileBuildPart, $info.FilePrivatePart) }
            }
        }
    }
    if ($choices.Count) { return ($choices | Sort-Object Version -Descending | Select-Object -First 1).Path }
    throw "No release Microsoft C runtime redistributables found for $arch. Install the matching Visual Studio C++ tools/redist component or pass -VCRedistDir <VS>/VC/Redist/MSVC/<version>."
}

function Get-MsvcRuntimeFiles([string[]]$Executables, [string]$Target, [string]$Override) {
    $folder = Find-MsvcRedist $Target $Override
    $pending = [Collections.Generic.Queue[string]]::new()
    $minimum = [version]'0.0'
    foreach ($file in $Executables) {
        $pending.Enqueue($file)
        $reader = [IO.BinaryReader]::new([IO.File]::OpenRead($file))
        try {
            $reader.BaseStream.Position = 0x3C
            $pe = $reader.ReadUInt32()
            $reader.BaseStream.Position = $pe + 26
            $linker = [version]::new($reader.ReadByte(), $reader.ReadByte())
            if ($linker -gt $minimum) { $minimum = $linker }
        } finally { $reader.Dispose() }
    }
    $files = @{}
    while ($pending.Count) {
        foreach ($name in @(Get-PeImports $pending.Dequeue())) {
            if ($name -notmatch '^(?:vcruntime|msvcp|concrt|vccorlib|vcomp|mfc|vcamp)\d.*\.dll$' -or $files.ContainsKey($name)) { continue }
            if ($script:MsvcRuntimeNames -notcontains $name) { throw "Unrecognized Microsoft runtime $name; update the packaging and uninstall allowlists before release." }
            $path = Join-Path $folder $name
            if (-not (Test-Path -LiteralPath $path -PathType Leaf)) { throw "Required Microsoft runtime $name is missing from the release CRT redistributable directory $folder." }
            Assert-PeTarget $path $Target
            $info = [Diagnostics.FileVersionInfo]::GetVersionInfo($path)
            $runtimeVersion = [version]::new($info.FileMajorPart, $info.FileMinorPart)
            if ($runtimeVersion -lt $minimum) { throw "Microsoft runtime $name ($runtimeVersion) predates the executable's linker toolset ($minimum). Use the matching or newer Visual Studio redistributables." }
            $files[$name] = $path
            $pending.Enqueue($path)
        }
    }
    return @($files.Keys | Sort-Object | ForEach-Object { $files[$_] })
}

# Resolve and check every existing ancestor before any destructive staging operation.
# A junction inside the output tree must never redirect cleanup outside that tree.
function Assert-PackagePath([string]$Path, [string]$Root) {
    $rootFull = [IO.Path]::GetFullPath($Root).TrimEnd([IO.Path]::DirectorySeparatorChar, [IO.Path]::AltDirectorySeparatorChar)
    $pathFull = [IO.Path]::GetFullPath($Path)
    if (-not $pathFull.StartsWith($rootFull + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) {
        throw "Packaging path '$pathFull' is not inside the output folder '$rootFull'."
    }
    $ancestor = $pathFull
    while ($ancestor) {
        if (Test-Path -LiteralPath $ancestor) {
            $item = Get-Item -LiteralPath $ancestor -Force
            if ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) { throw "Packaging refuses linked path: $ancestor" }
        }
        $parent = [IO.Path]::GetDirectoryName($ancestor)
        if ($parent -eq $ancestor) { break }
        $ancestor = $parent
    }
    return $pathFull
}

function Assert-PlainPackageTree([string]$Path) {
    if (-not (Test-Path -LiteralPath $Path)) { return }
    $pending = [Collections.Generic.Stack[string]]::new()
    $pending.Push([IO.Path]::GetFullPath($Path))
    while ($pending.Count) {
        $next = $pending.Pop()
        $item = Get-Item -LiteralPath $next -Force
        if ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) { throw "Packaging refuses linked content: $next" }
        if ($item.PSIsContainer) {
            foreach ($child in Get-ChildItem -LiteralPath $next -Force) { $pending.Push($child.FullName) }
        }
    }
}

function Reset-PackageDirectory([string]$Path, [string]$Root) {
    $safe = Assert-PackagePath $Path $Root
    Assert-PlainPackageTree $safe
    if (Test-Path -LiteralPath $safe) { Remove-Item -LiteralPath $safe -Recurse -Force }
    [void][IO.Directory]::CreateDirectory($safe)
}

function Remove-PackageDirectory([string]$Path, [string]$Root) {
    $safe = Assert-PackagePath $Path $Root
    Assert-PlainPackageTree $safe
    if (Test-Path -LiteralPath $safe) { Remove-Item -LiteralPath $safe -Recurse -Force }
}
