[CmdletBinding()]
param(
    [string]$Installer = (Join-Path $PSScriptRoot '..\target\dist\edolwave-installer-exe.exe'),
    [string]$InstallDirectory = (Join-Path $PSScriptRoot '..\target\installer-test'),
    [string]$RegistrationName = 'EdolWave',
    [string]$ExecutableName = 'edolwave.exe',
    [string]$AppId = '{C1E7C3EC-BA70-4575-8D0B-43FA62D8B9B0}'
)
$ErrorActionPreference = 'Stop'
$Installer = (Resolve-Path -LiteralPath $Installer).Path
$InstallDirectory = [IO.Path]::GetFullPath($InstallDirectory)
$expectedVersion = & (Join-Path $PSScriptRoot 'version.ps1')
$formats = Get-Content -LiteralPath (Join-Path $PSScriptRoot '..\installer\audio-formats.json') -Raw | ConvertFrom-Json
$extensions = @($formats.native) + @($formats.ffmpeg)
$progId = "$RegistrationName.Audio"
$capabilities = "Software\Edoli\$RegistrationName\Capabilities"
$application = "Software\Classes\Applications\$ExecutableName"
$uninstallKey = "Software\Microsoft\Windows\CurrentVersion\Uninstall\${AppId}_is1"

function Read-RegistryValue($Hive, [string]$Path, [string]$Name) {
    $key = $Hive.OpenSubKey($Path)
    if (-not $key) { return $null }
    try {
        if ($Name -notin $key.GetValueNames()) { return $null }
        [pscustomobject]@{ Kind = $key.GetValueKind($Name).ToString(); Value = $key.GetValue($Name) }
    } finally { $key.Dispose() }
}

function Assert-NoRegistration {
    foreach ($path in @("Software\Classes\$progId", $application, $capabilities, $uninstallKey)) {
        foreach ($hive in @([Microsoft.Win32.Registry]::CurrentUser, [Microsoft.Win32.Registry]::LocalMachine)) {
            $key = $hive.OpenSubKey($path)
            if ($key) { $key.Dispose(); throw "Existing registration at $path; refusing to overwrite an installed app." }
        }
    }
    if (Read-RegistryValue ([Microsoft.Win32.Registry]::CurrentUser) 'Software\RegisteredApplications' $RegistrationName) {
        throw 'An existing Default apps registration is present.'
    }
    foreach ($extension in $extensions) {
        if (Read-RegistryValue ([Microsoft.Win32.Registry]::CurrentUser) "Software\Classes\.$extension\OpenWithProgids" $progId) {
            throw "An existing .$extension association is present."
        }
    }
}

function Get-DefaultSnapshot {
    $state = foreach ($extension in $extensions) {
        $userChoice = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey("Software\Microsoft\Windows\CurrentVersion\Explorer\FileExts\.$extension\UserChoice")
        $choice = @()
        if ($userChoice) {
            try {
                $choice = @(foreach ($name in ($userChoice.GetValueNames() | Sort-Object)) {
                    [pscustomobject]@{ Name = $name; Kind = $userChoice.GetValueKind($name).ToString(); Value = $userChoice.GetValue($name) }
                })
            } finally { $userChoice.Dispose() }
        }
        [pscustomobject]@{
            Extension = $extension
            UserDefault = Read-RegistryValue ([Microsoft.Win32.Registry]::CurrentUser) "Software\Classes\.$extension" ''
            MachineDefault = Read-RegistryValue ([Microsoft.Win32.Registry]::LocalMachine) "Software\Classes\.$extension" ''
            UserChoice = $choice
        }
    }
    ConvertTo-Json -InputObject @($state) -Depth 8 -Compress
}

function Assert-Associations([string[]]$Selected) {
    foreach ($extension in $extensions) {
        $openWith = Read-RegistryValue ([Microsoft.Win32.Registry]::CurrentUser) "Software\Classes\.$extension\OpenWithProgids" $progId
        $supported = Read-RegistryValue ([Microsoft.Win32.Registry]::CurrentUser) "$application\SupportedTypes" ".$extension"
        $capability = Read-RegistryValue ([Microsoft.Win32.Registry]::CurrentUser) "$capabilities\FileAssociations" ".$extension"
        if ($extension -in $Selected) {
            if (-not $openWith -or -not $supported -or $capability.Value -cne $progId) {
                throw "Missing selected association: .$extension"
            }
        } elseif ($openWith -or $supported -or $capability) {
            throw "Unselected association was registered: .$extension"
        }
    }
    $registered = Read-RegistryValue ([Microsoft.Win32.Registry]::CurrentUser) 'Software\RegisteredApplications' $RegistrationName
    $command = Read-RegistryValue ([Microsoft.Win32.Registry]::CurrentUser) "Software\Classes\$progId\shell\open\command" ''
    if ($Selected.Count) {
        if ($registered.Value -cne $capabilities) { throw 'Default apps registration is incorrect.' }
        $expectedCommand = '"' + (Join-Path $InstallDirectory $ExecutableName) + '" "%1"'
        if ($command.Value -cne $expectedCommand) { throw 'File opening command is not correctly quoted.' }
    } elseif ($registered -or $command) {
        throw 'Disabling all associations left app registration behind.'
    }
    if ((Get-DefaultSnapshot) -cne $script:defaultSnapshot) { throw 'Installer changed an existing default app or UserChoice.' }
}

Assert-NoRegistration
if (Test-Path -LiteralPath $InstallDirectory) { throw "Install test directory already exists: $InstallDirectory" }
$defaultSnapshot = Get-DefaultSnapshot
$allTasks = ($extensions | ForEach-Object { "fileassociations\$_" }) -join ','
$cases = @(
    @{ Name = 'default selection'; Selected = @($formats.native); Tasks = $null },
    @{ Name = 'all formats'; Selected = $extensions; Tasks = $allTasks },
    @{ Name = 'upgrade to WAV and MP3 only'; Selected = @('wav', 'mp3'); Tasks = 'fileassociations\wav,fileassociations\mp3' },
    @{ Name = 'upgrade with associations disabled'; Selected = @(); Tasks = '' },
    @{ Name = 're-enable all formats before uninstall'; Selected = $extensions; Tasks = $allTasks }
)
try {
    foreach ($case in $cases) {
        $arguments = @('/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART', '/SP-', '/CURRENTUSER',
            '/NOICONS', ('/DIR="' + $InstallDirectory + '"'))
        if ($null -ne $case.Tasks) { $arguments += "/TASKS=$($case.Tasks)" }
        $process = Start-Process -FilePath $Installer -WindowStyle Hidden -Wait -PassThru -ArgumentList $arguments
        if ($process.ExitCode -ne 0) { throw "Install failed ($($case.Name)): $($process.ExitCode)" }
        if ((Get-Item -LiteralPath (Join-Path $InstallDirectory $ExecutableName)).VersionInfo.ProductVersion -cne $expectedVersion) {
            throw 'Installed executable version does not match Cargo.toml.'
        }
        foreach ($dll in @('vcruntime140.dll', 'vcruntime140_1.dll', 'msvcp140.dll', 'concrt140.dll')) {
            if (-not (Test-Path -LiteralPath (Join-Path $InstallDirectory $dll))) { throw "Missing runtime: $dll" }
        }
        Assert-Associations -Selected $case.Selected
        Write-Output "PASS: $($case.Name)"
    }
} finally {
    $uninstaller = Join-Path $InstallDirectory 'unins000.exe'
    if (Test-Path -LiteralPath $uninstaller) {
        $process = Start-Process -FilePath $uninstaller -WindowStyle Hidden -Wait -PassThru -ArgumentList @(
            '/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART')
        if ($process.ExitCode -ne 0) { throw "Uninstall failed: $($process.ExitCode)" }
    }
}
Assert-NoRegistration
Assert-Associations -Selected @()
if (Test-Path -LiteralPath (Join-Path $InstallDirectory $ExecutableName)) { throw 'Uninstall left the executable behind.' }
Write-Output 'PASS: uninstall cleanup and preservation of existing defaults'
