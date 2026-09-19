#define AppName "Litematica Preview"
#define AppVersion "0.1.0"

[Setup]
AppId={{CB97B04E-3CEC-4D9A-A9A9-980C337A2807}
AppName={#AppName}
AppVersion={#AppVersion}
DefaultDirName={localappdata}\Programs\LitematicaPreview
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0
DefaultGroupName={#AppName}
DisableProgramGroupPage=yes
UninstallDisplayIcon={app}\LitematicaPreview.exe
SetupIconFile=..\Assets\app.ico
LicenseFile=..\LICENSE
OutputDir=..\artifacts
OutputBaseFilename=LitematicaPreview-{#AppVersion}-win-x64-setup
Compression=lzma2
SolidCompression=yes
ChangesAssociations=yes

[Files]
Source: "..\artifacts\win-x64\*"; DestDir: "{app}"; Flags: ignoreversion recursesubdirs createallsubdirs

[Icons]
Name: "{group}\{#AppName}"; Filename: "{app}\LitematicaPreview.exe"

[Run]
Filename: "{app}\LitematicaPreview.exe"; Parameters: "--register"; Flags: runhidden waituntilterminated
Filename: "{app}\LitematicaPreview.exe"; Description: "Open {#AppName}"; Flags: nowait postinstall skipifsilent

[UninstallRun]
Filename: "{app}\LitematicaPreview.exe"; Parameters: "--unregister"; Flags: runhidden waituntilterminated; RunOnceId: "RemoveFileAssociations"
