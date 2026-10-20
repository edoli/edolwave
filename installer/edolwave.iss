; Build with tools/package.ps1; Cargo.toml is the source of the version.
#ifndef AppVersion
  #error AppVersion is required
#endif
#ifndef SourceExe
  #error SourceExe is required
#endif
#ifndef OutputPath
  #error OutputPath is required
#endif
#ifndef ProjectRoot
  #error ProjectRoot is required
#endif
#ifndef AssociationsFile
  #error AssociationsFile is required
#endif

[Setup]
AppId={{C1E7C3EC-BA70-4575-8D0B-43FA62D8B9B0}
AppName=EdolWave
AppVersion={#AppVersion}
AppPublisher=Edoli
DefaultDirName={autopf}\EdolWave
DefaultGroupName=EdolWave
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
UsePreviousPrivileges=yes
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0
OutputDir={#OutputPath}
OutputBaseFilename=edolwave-installer-exe
SetupIconFile={#ProjectRoot}\icons\app.ico
LicenseFile={#ProjectRoot}\LICENSE
UninstallDisplayIcon={app}\edolwave.exe
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern
ShowLanguageDialog=no
CloseApplications=no
RestartApplications=no
SetupLogging=yes
ChangesAssociations=yes

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "startmenuicon"; Description: "Create Start Menu shortcuts"; GroupDescription: "{cm:AdditionalIcons}"
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked
Name: "fileassociations"; Description: "Add EdolWave to Open with and Default apps"; GroupDescription: "Audio file associations"
#include AssociationsFile

[Registry]
Root: HKA; Subkey: "Software\Edoli\EdolWave"; ValueType: string; ValueName: "InstallLocation"; ValueData: "{app}"; Flags: uninsdeletevalue uninsdeletekeyifempty
Root: HKA; Subkey: "Software\Classes\EdolWave.Audio"; Check: not HasAudioAssociations; Flags: deletekey
Root: HKA; Subkey: "Software\Classes\Applications\edolwave.exe"; Check: not HasAudioAssociations; Flags: deletekey
Root: HKA; Subkey: "Software\Edoli\EdolWave\Capabilities"; Check: not HasAudioAssociations; Flags: deletekey
Root: HKA; Subkey: "Software\RegisteredApplications"; ValueType: none; ValueName: "EdolWave"; Check: not HasAudioAssociations; Flags: deletevalue
Root: HKA; Subkey: "Software\Classes\EdolWave.Audio"; ValueType: string; ValueName: ""; ValueData: "EdolWave audio file"; Check: HasAudioAssociations; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Classes\EdolWave.Audio\DefaultIcon"; ValueType: string; ValueName: ""; ValueData: """{app}\edolwave.exe"",0"; Check: HasAudioAssociations
Root: HKA; Subkey: "Software\Classes\EdolWave.Audio\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\edolwave.exe"" ""%1"""; Check: HasAudioAssociations
Root: HKA; Subkey: "Software\Classes\Applications\edolwave.exe"; ValueType: string; ValueName: "FriendlyAppName"; ValueData: "EdolWave"; Check: HasAudioAssociations; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Classes\Applications\edolwave.exe\DefaultIcon"; ValueType: string; ValueName: ""; ValueData: """{app}\edolwave.exe"",0"; Check: HasAudioAssociations
Root: HKA; Subkey: "Software\Classes\Applications\edolwave.exe\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\edolwave.exe"" ""%1"""; Check: HasAudioAssociations
Root: HKA; Subkey: "Software\Edoli\EdolWave\Capabilities"; ValueType: string; ValueName: "ApplicationName"; ValueData: "EdolWave"; Check: HasAudioAssociations; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Edoli\EdolWave\Capabilities"; ValueType: string; ValueName: "ApplicationDescription"; ValueData: "Audio player with waveform navigation, playback speed and repeat."; Check: HasAudioAssociations
Root: HKA; Subkey: "Software\Edoli\EdolWave\Capabilities"; ValueType: string; ValueName: "ApplicationIcon"; ValueData: """{app}\edolwave.exe"",0"; Check: HasAudioAssociations
Root: HKA; Subkey: "Software\RegisteredApplications"; ValueType: string; ValueName: "EdolWave"; ValueData: "Software\Edoli\EdolWave\Capabilities"; Check: HasAudioAssociations; Flags: uninsdeletevalue uninsdeletekeyifempty

[Files]
Source: "{#SourceExe}"; DestDir: "{app}"; DestName: "edolwave.exe"; Flags: ignoreversion
Source: "{#ExtractFilePath(SourceExe)}*.dll"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#ProjectRoot}\LICENSE"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#ProjectRoot}\README.md"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\EdolWave\EdolWave"; Filename: "{app}\edolwave.exe"; Tasks: startmenuicon; AppUserModelID: "dev.edoli.edolwave"
Name: "{autoprograms}\EdolWave\Uninstall EdolWave"; Filename: "{uninstallexe}"; Tasks: startmenuicon
Name: "{autodesktop}\EdolWave"; Filename: "{app}\edolwave.exe"; Tasks: desktopicon; AppUserModelID: "dev.edoli.edolwave"

[Run]
Filename: "{app}\edolwave.exe"; Description: "{cm:LaunchProgram,EdolWave}"; Flags: nowait postinstall skipifsilent runasoriginaluser
Filename: "{code:GetDefaultAppsURI}"; Description: "Choose EdolWave as your default audio player in Windows Settings"; Check: HasAudioAssociations; Flags: shellexec postinstall skipifsilent runasoriginaluser

[Code]
#include "associations-ui.iss"

function HasAudioAssociations: Boolean;
begin
  Result := WizardIsTaskSelected('fileassociations');
end;

function GetDefaultAppsURI(Param: string): string;
var
  Version: TWindowsVersion;
begin
  Result := 'ms-settings:defaultapps';
  GetWindowsVersionEx(Version);
  if Version.Build >= 22000 then begin
    if IsAdminInstallMode then
      Result := Result + '?registeredAppMachine=EdolWave'
    else
      Result := Result + '?registeredAppUser=EdolWave';
  end;
end;

function CreateFileW(FileName: string; DesiredAccess, ShareMode: Cardinal;
  SecurityAttributes: Integer; CreationDisposition, FlagsAndAttributes: Cardinal;
  TemplateFile: THandle): THandle;
  external 'CreateFileW@kernel32.dll stdcall';
function CloseHandle(Handle: THandle): Boolean;
  external 'CloseHandle@kernel32.dll stdcall';

function PrepareToInstall(var NeedsRestart: Boolean): string;
var
  Handle: THandle;
  ExePath: string;
begin
  Result := '';
  ExePath := ExpandConstant('{app}\edolwave.exe');
  if FileExists(ExePath) then begin
    Handle := CreateFileW(ExePath, $40000000, 7, 0, 3, $80, 0);
    if Handle = THandle(-1) then
      Result := 'Close all EdolWave windows before installing the update.'
    else
      CloseHandle(Handle);
  end;
end;
