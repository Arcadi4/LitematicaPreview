; Litematica Preview per-user installer.
;
; Declarative only: xtask compiles this after signing the executable and
; supplies AppVersion, NumericVersion, StageDir, and OutputDir with /D.
;
; The association below is optional and entirely per-user. It registers a
; stable ProgID distinct from earlier installations and never writes
; HKEY_LOCAL_MACHINE or the protected UserChoice key.

#ifndef AppVersion
  #error AppVersion must be defined
#endif
#ifndef NumericVersion
  #define NumericVersion "0.0.0.0"
#endif
#ifndef StageDir
  #error StageDir must be defined
#endif
#ifndef LicenseFile
  #error LicenseFile must be defined
#endif
#ifndef OutputDir
  #error OutputDir must be defined
#endif

#define AppName "Litematica Preview"
#define AppPublisher "Arcadi4"
#define AppUrl "https://github.com/Arcadi4/LitematicaPreview"
#define ProgId "LitematicaPreview.Native.Schematic"
#define RegisteredAppName "Litematica Preview Native"

[Setup]
AppId={{8F9A7D3E-2B4C-4D6E-9F10-2A3B4C5D6E7F}
AppName={#AppName}
AppVersion={#AppVersion}
AppVerName={#AppName} {#AppVersion}
AppPublisher={#AppPublisher}
AppPublisherURL={#AppUrl}
AppSupportURL={#AppUrl}/issues
AppUpdatesURL={#AppUrl}/releases
VersionInfoVersion={#NumericVersion}
VersionInfoCompany={#AppPublisher}
VersionInfoDescription={#AppName} Setup
VersionInfoProductName={#AppName}
VersionInfoProductVersion={#NumericVersion}
VersionInfoProductTextVersion={#AppVersion}
DefaultDirName={localappdata}\Programs\LitematicaPreview
DefaultGroupName={#AppName}
DisableDirPage=no
DisableProgramGroupPage=yes
DisableWelcomePage=no
UsePreviousAppDir=yes
AllowNoIcons=yes
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0
ChangesAssociations=yes
CloseApplications=yes
RestartApplications=no
Compression=lzma2/ultra64
SolidCompression=yes
WizardStyle=modern
OutputDir={#OutputDir}
OutputBaseFilename=LitematicaPreview-{#AppVersion}-x86_64-setup
SetupIconFile=..\..\Assets\app.ico
UninstallDisplayIcon={app}\LitematicaPreview.exe
UninstallDisplayName={#AppName}
LicenseFile={#LicenseFile}
CreateUninstallRegKey=yes
SetupLogging=yes

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked
Name: "associate"; Description: "Associate Minecraft schematic files with {#AppName}"; GroupDescription: "File associations:"; Flags: unchecked

[Files]
Source: "{#StageDir}\*"; DestDir: "{app}"; Flags: ignoreversion recursesubdirs createallsubdirs

[Icons]
Name: "{autoprograms}\{#AppName}"; Filename: "{app}\LitematicaPreview.exe"
Name: "{autodesktop}\{#AppName}"; Filename: "{app}\LitematicaPreview.exe"; Tasks: desktopicon

[Run]
Filename: "{app}\LitematicaPreview.exe"; Description: "{cm:LaunchProgram,{#AppName}}"; Flags: nowait postinstall skipifsilent

[Registry]
; Exclusive ProgID and capability keys: nothing else owns these names.
Root: HKCU; Subkey: "Software\Classes\{#ProgId}"; ValueType: string; ValueName: ""; ValueData: "Minecraft schematic"; Tasks: associate
Root: HKCU; Subkey: "Software\Classes\{#ProgId}\DefaultIcon"; ValueType: string; ValueName: ""; ValueData: """{app}\LitematicaPreview.exe"",0"; Tasks: associate
Root: HKCU; Subkey: "Software\Classes\{#ProgId}\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\LitematicaPreview.exe"" --open ""%1"""; Tasks: associate
Root: HKCU; Subkey: "Software\LitematicaPreview.Native\Capabilities"; ValueType: string; ValueName: "ApplicationName"; ValueData: "{#AppName}"; Tasks: associate
Root: HKCU; Subkey: "Software\LitematicaPreview.Native\Capabilities"; ValueType: string; ValueName: "ApplicationDescription"; ValueData: "View Minecraft schematics and structures offline."; Tasks: associate
Root: HKCU; Subkey: "Software\LitematicaPreview.Native\Capabilities\FileAssociations"; ValueType: string; ValueName: ".litematic"; ValueData: "{#ProgId}"; Tasks: associate
Root: HKCU; Subkey: "Software\LitematicaPreview.Native\Capabilities\FileAssociations"; ValueType: string; ValueName: ".schem"; ValueData: "{#ProgId}"; Tasks: associate
Root: HKCU; Subkey: "Software\LitematicaPreview.Native\Capabilities\FileAssociations"; ValueType: string; ValueName: ".schematic"; ValueData: "{#ProgId}"; Tasks: associate
Root: HKCU; Subkey: "Software\LitematicaPreview.Native\Capabilities\FileAssociations"; ValueType: string; ValueName: ".nbt"; ValueData: "{#ProgId}"; Tasks: associate
Root: HKCU; Subkey: "Software\LitematicaPreview.Native\Capabilities\FileAssociations"; ValueType: string; ValueName: ".snbt"; ValueData: "{#ProgId}"; Tasks: associate
Root: HKCU; Subkey: "Software\LitematicaPreview.Native\Capabilities\FileAssociations"; ValueType: string; ValueName: ".mcstructure"; ValueData: "{#ProgId}"; Tasks: associate
Root: HKCU; Subkey: "Software\LitematicaPreview.Native\Capabilities\FileAssociations"; ValueType: string; ValueName: ".mca"; ValueData: "{#ProgId}"; Tasks: associate
Root: HKCU; Subkey: "Software\LitematicaPreview.Native\Capabilities\FileAssociations"; ValueType: string; ValueName: ".nusn"; ValueData: "{#ProgId}"; Tasks: associate

; Shared keys: only add our values. CurUninstallStepChanged removes them only
; while they still belong to this installation.
Root: HKCU; Subkey: "Software\RegisteredApplications"; ValueType: string; ValueName: "{#RegisteredAppName}"; ValueData: "Software\LitematicaPreview.Native\Capabilities"; Tasks: associate
Root: HKCU; Subkey: "Software\Classes\.litematic\OpenWithProgids"; ValueType: binary; ValueName: "{#ProgId}"; Tasks: associate
Root: HKCU; Subkey: "Software\Classes\.schem\OpenWithProgids"; ValueType: binary; ValueName: "{#ProgId}"; Tasks: associate
Root: HKCU; Subkey: "Software\Classes\.schematic\OpenWithProgids"; ValueType: binary; ValueName: "{#ProgId}"; Tasks: associate
Root: HKCU; Subkey: "Software\Classes\.nbt\OpenWithProgids"; ValueType: binary; ValueName: "{#ProgId}"; Tasks: associate
Root: HKCU; Subkey: "Software\Classes\.snbt\OpenWithProgids"; ValueType: binary; ValueName: "{#ProgId}"; Tasks: associate
Root: HKCU; Subkey: "Software\Classes\.mcstructure\OpenWithProgids"; ValueType: binary; ValueName: "{#ProgId}"; Tasks: associate
Root: HKCU; Subkey: "Software\Classes\.mca\OpenWithProgids"; ValueType: binary; ValueName: "{#ProgId}"; Tasks: associate
Root: HKCU; Subkey: "Software\Classes\.nusn\OpenWithProgids"; ValueType: binary; ValueName: "{#ProgId}"; Tasks: associate

; Default handlers are claimed only when nothing else owns the extension and
; Explorer has no UserChoice for it.
Root: HKCU; Subkey: "Software\Classes\.litematic"; ValueType: string; ValueName: ""; ValueData: "{#ProgId}"; Tasks: associate; Check: CanClaimExtension('.litematic')
Root: HKCU; Subkey: "Software\Classes\.schem"; ValueType: string; ValueName: ""; ValueData: "{#ProgId}"; Tasks: associate; Check: CanClaimExtension('.schem')
Root: HKCU; Subkey: "Software\Classes\.schematic"; ValueType: string; ValueName: ""; ValueData: "{#ProgId}"; Tasks: associate; Check: CanClaimExtension('.schematic')
Root: HKCU; Subkey: "Software\Classes\.nbt"; ValueType: string; ValueName: ""; ValueData: "{#ProgId}"; Tasks: associate; Check: CanClaimExtension('.nbt')
Root: HKCU; Subkey: "Software\Classes\.snbt"; ValueType: string; ValueName: ""; ValueData: "{#ProgId}"; Tasks: associate; Check: CanClaimExtension('.snbt')
Root: HKCU; Subkey: "Software\Classes\.mcstructure"; ValueType: string; ValueName: ""; ValueData: "{#ProgId}"; Tasks: associate; Check: CanClaimExtension('.mcstructure')
Root: HKCU; Subkey: "Software\Classes\.mca"; ValueType: string; ValueName: ""; ValueData: "{#ProgId}"; Tasks: associate; Check: CanClaimExtension('.mca')
Root: HKCU; Subkey: "Software\Classes\.nusn"; ValueType: string; ValueName: ""; ValueData: "{#ProgId}"; Tasks: associate; Check: CanClaimExtension('.nusn')

[Code]
const
  ProgId = '{#ProgId}';
  CapabilitiesKey = 'Software\LitematicaPreview.Native\Capabilities';
  RegisteredAppName = '{#RegisteredAppName}';
  ExtensionCount = 8;

function ExtensionAt(Index: Integer): String;
begin
  if Index = 0 then
    Result := '.litematic'
  else if Index = 1 then
    Result := '.schem'
  else if Index = 2 then
    Result := '.schematic'
  else if Index = 3 then
    Result := '.nbt'
  else if Index = 4 then
    Result := '.snbt'
  else if Index = 5 then
    Result := '.mcstructure'
  else if Index = 6 then
    Result := '.mca'
  else
    Result := '.nusn';
end;

function CanClaimExtension(const Extension: String): Boolean;
var
  Value: String;
begin
  Result := True;
  if RegQueryStringValue(
    HKCU,
    'Software\Microsoft\Windows\CurrentVersion\Explorer\FileExts\' + Extension + '\UserChoice',
    'ProgId',
    Value
  ) then
  begin
    Result := False;
    exit;
  end;
  if RegQueryStringValue(HKCU, 'Software\Classes\' + Extension, '', Value) and (Value <> '') then
  begin
    Result := False;
    exit;
  end;
  if RegQueryStringValue(HKCR, Extension, '', Value) and (Value <> '') then
    Result := False;
end;

procedure RemoveAssociations;
var
  Expected: String;
  Index: Integer;
  Value: String;
begin
  // A second copy can share this stable ProgID. Remove registrations only
  // while the open command still names this installation.
  Expected := '"' + ExpandConstant('{app}') + '\LitematicaPreview.exe" --open "%1"';
  if not RegQueryStringValue(
    HKCU,
    'Software\Classes\' + ProgId + '\shell\open\command',
    '',
    Value
  ) or (CompareText(Value, Expected) <> 0) then
    exit;

  if RegQueryStringValue(HKCU, 'Software\RegisteredApplications', RegisteredAppName, Value) and
     (CompareText(Value, CapabilitiesKey) = 0) then
    RegDeleteValue(HKCU, 'Software\RegisteredApplications', RegisteredAppName);

  for Index := 0 to ExtensionCount - 1 do
  begin
    if RegQueryStringValue(HKCU, 'Software\Classes\' + ExtensionAt(Index), '', Value) and
       (CompareText(Value, ProgId) = 0) then
      RegDeleteValue(HKCU, 'Software\Classes\' + ExtensionAt(Index), '');
    RegDeleteValue(
      HKCU,
      'Software\Classes\' + ExtensionAt(Index) + '\OpenWithProgids',
      ProgId
    );
  end;

  DelTree('HKCU\Software\Classes\' + ProgId, True, True, False);
  DelTree('HKCU\Software\LitematicaPreview.Native', True, True, False);
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
begin
  if CurUninstallStep = usPostUninstall then
    RemoveAssociations;
end;
