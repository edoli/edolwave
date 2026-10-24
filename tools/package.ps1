[CmdletBinding()]
param(
    [string]$Compiler = $env:INNO_SETUP_COMPILER,
    [string]$OutputDirectory = (Join-Path $PSScriptRoot '..\target\dist'),
    [string]$SourceExe = (Join-Path $PSScriptRoot '..\target\release-dist\edolwave.exe'),
    [switch]$NoBuild
)
$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path -Parent $PSScriptRoot
$version = & (Join-Path $PSScriptRoot 'version.ps1')
if (-not $Compiler) {
    $candidates = @(
        (Join-Path $projectRoot 'target\tools\InnoSetup\ISCC.exe'),
        'C:\Program Files (x86)\Inno Setup 6\ISCC.exe',
        'C:\Program Files\Inno Setup 7\ISCC.exe'
    )
    $Compiler = $candidates | Where-Object { Test-Path -LiteralPath $_ } | Select-Object -First 1
}
if (-not $Compiler -or -not (Test-Path -LiteralPath $Compiler)) {
    throw 'Install Inno Setup first: powershell -File tools/install_inno.ps1'
}
$Compiler = [IO.Path]::GetFullPath($Compiler)
$SourceExe = [IO.Path]::GetFullPath($SourceExe)
$OutputDirectory = [IO.Path]::GetFullPath($OutputDirectory)
Push-Location $projectRoot
try {
    if (-not $NoBuild) {
        & cargo build --profile release-dist --locked
        if ($LASTEXITCODE -ne 0) { throw 'Release build failed' }
    }
    $binary = Get-Item -LiteralPath $SourceExe
    if ($binary.VersionInfo.ProductVersion -cne $version) {
        throw "EXE version '$($binary.VersionInfo.ProductVersion)' does not match Cargo.toml version '$version'."
    }
    # Reject ARM64/x86 inputs even when a caller overrides SourceExe.
    $stream = [IO.File]::OpenRead($SourceExe)
    $reader = [IO.BinaryReader]::new($stream)
    try {
        $stream.Position = 0x3c
        $peOffset = $reader.ReadInt32()
        $stream.Position = $peOffset
        if ($reader.ReadUInt32() -ne 0x00004550 -or $reader.ReadUInt16() -ne 0x8664) {
            throw 'Windows packaging requires an x86_64 MSVC executable.'
        }
    } finally { $reader.Dispose() }

    New-Item -ItemType Directory -Path $OutputDirectory -Force | Out-Null
    # Remove only obsolete generated artifacts left by earlier local packaging.
    Get-ChildItem -LiteralPath $OutputDirectory -File | Where-Object {
        $_.Name -match '^SHA256SUMS(?:-(windows|mac|linux))?\.txt$' -or
        $_.Name -eq 'edot-update.txt' -or $_.Name -match '^Edot-Setup-[0-9]+\.[0-9]+\.[0-9]+-windows-x64\.exe$'
    } | ForEach-Object { Remove-Item -LiteralPath $_.FullName -Force }

    # Use a new stage each time so stale files cannot enter a later package.
    $stage = Join-Path $projectRoot ('target\package-' + [guid]::NewGuid().ToString('N'))
    New-Item -ItemType Directory -Path $stage | Out-Null
    try {
        Copy-Item -LiteralPath $SourceExe -Destination (Join-Path $stage 'edolwave.exe')
        $associationsFile = Join-Path $stage 'file-associations.iss'
        & (Join-Path $PSScriptRoot 'generate-associations.ps1') -OutputFile $associationsFile
        & $Compiler "/DAppVersion=$version" "/DSourceExe=$stage\edolwave.exe" "/DOutputPath=$OutputDirectory" "/DProjectRoot=$projectRoot" "/DAssociationsFile=$associationsFile" (Join-Path $projectRoot 'installer\edolwave.iss')
        if ($LASTEXITCODE -ne 0) { throw 'Installer compilation failed' }
        Remove-Item -LiteralPath $associationsFile
        $installer = Join-Path $OutputDirectory 'edolwave-installer-exe.exe'
        $portable = Join-Path $OutputDirectory 'edolwave-windows.zip'
        Compress-Archive -Path (Join-Path $stage '*') -DestinationPath $portable -Force
        $checksums = @{}
        foreach ($file in @($installer, $portable)) {
            $checksums[[IO.Path]::GetFileName($file)] = (Get-FileHash -LiteralPath $file -Algorithm SHA256).Hash.ToLowerInvariant()
        }
        [IO.File]::WriteAllText((Join-Path $OutputDirectory 'package-checksums-windows.json'), ($checksums | ConvertTo-Json), [Text.Encoding]::ASCII)
        Get-Item -LiteralPath $installer, $portable | Select-Object Name, Length
    } finally {
        # stage is an absolute, newly created child of this project's target directory.
        $targetRoot = [IO.Path]::GetFullPath((Join-Path $projectRoot 'target')) + [IO.Path]::DirectorySeparatorChar
        $resolvedStage = [IO.Path]::GetFullPath($stage)
        if (-not $resolvedStage.StartsWith($targetRoot, [StringComparison]::OrdinalIgnoreCase)) {
            throw "Refusing to remove stage outside target: $resolvedStage"
        }
        Remove-Item -LiteralPath $resolvedStage -Recurse -Force
    }
} finally { Pop-Location }
