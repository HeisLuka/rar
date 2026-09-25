#define MyAppName "Chaptera PUB Reader"

#ifndef MyAppVersion
  #define MyAppVersion "0.1.0-preview"
#endif

#ifndef SourceDir
  #define SourceDir "..\..\stage\chaptera-reader"
#endif

#ifndef OutputDir
  #define OutputDir "..\..\dist"
#endif

[Setup]
AppId={{9B7E0F8A-3F30-4A2C-AE4B-5E84D7C1562A}
AppName={#MyAppName}
AppVersion={#MyAppVersion}
AppVerName={#MyAppName} {#MyAppVersion}
DefaultDirName={localappdata}\Programs\{#MyAppName}
DefaultGroupName={#MyAppName}
DisableProgramGroupPage=yes
OutputDir={#OutputDir}
OutputBaseFilename=Chaptera-Reader-Setup-{#MyAppVersion}
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
UninstallDisplayIcon={app}\chaptera-reader.exe
ChangesAssociations=yes
CloseApplications=yes
SetupLogging=yes

[Files]
Source: "{#SourceDir}\Chaptera-Reader.exe"; DestDir: "{app}"; DestName: "chaptera-reader.exe"; Flags: ignoreversion
Source: "{#SourceDir}\README.md"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{group}\Chaptera PUB Reader"; Filename: "{app}\chaptera-reader.exe"

[Registry]
Root: HKCU; Subkey: "Software\Classes\Chaptera.PUB.Reader"; ValueType: string; ValueName: ""; ValueData: "Microsoft Publisher Document"; Flags: uninsdeletekey
Root: HKCU; Subkey: "Software\Classes\Chaptera.PUB.Reader\DefaultIcon"; ValueType: string; ValueName: ""; ValueData: """{app}\chaptera-reader.exe"",0"
Root: HKCU; Subkey: "Software\Classes\Chaptera.PUB.Reader\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\chaptera-reader.exe"" ""%1"""
Root: HKCU; Subkey: "Software\Classes\Applications\chaptera-reader.exe"; ValueType: string; ValueName: "FriendlyAppName"; ValueData: "Chaptera PUB Reader"; Flags: uninsdeletekey
Root: HKCU; Subkey: "Software\Classes\Applications\chaptera-reader.exe\SupportedTypes"; ValueType: string; ValueName: ".pub"; ValueData: ""
Root: HKCU; Subkey: "Software\Classes\Applications\chaptera-reader.exe\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\chaptera-reader.exe"" ""%1"""

[Run]
Filename: "{sys}\reg.exe"; Parameters: "ADD ""HKCU\Software\Classes\.pub\OpenWithProgids"" /v ""Chaptera.PUB.Reader"" /t REG_NONE /f"; Flags: runhidden waituntilterminated

[UninstallRun]
Filename: "{sys}\reg.exe"; Parameters: "DELETE ""HKCU\Software\Classes\.pub\OpenWithProgids"" /v ""Chaptera.PUB.Reader"" /f"; Flags: runhidden waituntilterminated
