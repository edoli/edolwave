[CmdletBinding()]
param([string]$Tag)
$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path -Parent $PSScriptRoot
$metadata = & cargo metadata --no-deps --format-version 1 --manifest-path (Join-Path $projectRoot 'Cargo.toml')
if ($LASTEXITCODE -ne 0) { throw 'Cannot read Cargo.toml metadata' }
$package = ($metadata | ConvertFrom-Json).packages | Where-Object { $_.name -eq 'edolwave' }
$version = $package.version
if ($version -cnotmatch '^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$' -or
    @($version.Split('.') | Where-Object { [long]$_ -gt 65535 }).Count -ne 0) {
    throw 'Version must be major.minor.patch, with each component <= 65535 for Windows packaging.'
}
if ($Tag -and $Tag -cne "v$version") {
    throw "Tag '$Tag' does not match Cargo.toml version '$version' (expected v$version)."
}
$version
