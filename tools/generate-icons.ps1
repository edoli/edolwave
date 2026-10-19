# Regenerate the committed PNG and multi-resolution ICO from icon.svg.
# Requires ImageMagick 7 (magick) on PATH; normal builds use the committed ICO.
[CmdletBinding()]
param()
$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path -Parent $PSScriptRoot
if (-not (Get-Command magick -ErrorAction SilentlyContinue)) {
    throw 'Install ImageMagick 7 and add magick to PATH to regenerate icons.'
}
$icons = Join-Path $projectRoot 'icons'
New-Item -ItemType Directory -Path $icons -Force | Out-Null
& magick -background none (Join-Path $projectRoot 'icon.svg') (Join-Path $icons 'icon.png')
if ($LASTEXITCODE -ne 0) { throw 'SVG rasterization failed' }
& magick (Join-Path $icons 'icon.png') -define icon:auto-resize=256,128,64,48,32,24,16 (Join-Path $icons 'app.ico')
if ($LASTEXITCODE -ne 0) { throw 'ICO generation failed' }
