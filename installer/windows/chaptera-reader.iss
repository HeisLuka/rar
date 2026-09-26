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
AppId={{5D0E0D1E-DF1D-49E0-8A43-95F1778BFA21}
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
UninstallDisplayIcon={app}\current\chaptera-reader.exe
ChangesAssociations=yes
CloseApplications=yes
SetupLogging=yes

[Dirs]
Name: "{app}\current"
Name: "{app}\.staging"
Name: "{app}\.rollback"

[Files]
Source: "{#SourceDir}\Chaptera-Reader.exe"; DestDir: "{app}\current"; DestName: "chaptera-reader.exe"; Flags: ignoreversion
Source: "{#SourceDir}\README.md"; DestDir: "{app}\current"; Flags: ignoreversion

[Icons]
Name: "{group}\Chaptera PUB Reader"; Filename: "{app}\current\chaptera-reader.exe"

[Registry]
Root: HKCU; Subkey: "Software\Classes\Chaptera.PUB.Reader"; ValueType: string; ValueName: ""; ValueData: "Microsoft Publisher Document"; Flags: uninsdeletekey
Root: HKCU; Subkey: "Software\Classes\Chaptera.PUB.Reader\DefaultIcon"; ValueType: string; ValueName: ""; ValueData: """{app}\current\chaptera-reader.exe"",0"
Root: HKCU; Subkey: "Software\Classes\Chaptera.PUB.Reader\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\current\chaptera-reader.exe"" ""%1"""
Root: HKCU; Subkey: "Software\Classes\Applications\chaptera-reader.exe"; ValueType: string; ValueName: "FriendlyAppName"; ValueData: "Chaptera PUB Reader"; Flags: uninsdeletekey
Root: HKCU; Subkey: "Software\Classes\Applications\chaptera-reader.exe\SupportedTypes"; ValueType: string; ValueName: ".pub"; ValueData: ""
Root: HKCU; Subkey: "Software\Classes\Applications\chaptera-reader.exe\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\current\chaptera-reader.exe"" ""%1"""

[Run]
Filename: "{sys}\reg.exe"; Parameters: "ADD ""HKCU\Software\Classes\.pub\OpenWithProgids"" /v ""Chaptera.PUB.Reader"" /t REG_NONE /f"; Flags: runhidden waituntilterminated

[UninstallRun]
Filename: "{sys}\reg.exe"; Parameters: "DELETE ""HKCU\Software\Classes\.pub\OpenWithProgids"" /v ""Chaptera.PUB.Reader"" /f"; Flags: runhidden waituntilterminated

[UninstallDelete]
Type: filesandordirs; Name: "{app}\current"
Type: filesandordirs; Name: "{app}\.staging"
Type: filesandordirs; Name: "{app}\.rollback"
Type: files; Name: "{app}\update-journal.json"
Type: files; Name: "{app}\update-journal.json.next"
Type: files; Name: "{app}\update-journal.json.prev"
Type: files; Name: "{app}\.chaptera-install.lock"

[Code]
procedure CurStepChanged(CurStep: TSetupStep);
var
  CurrentReader: String;
  LegacyReader: String;
  LegacyReadme: String;
begin
  if CurStep = ssPostInstall then
  begin
    CurrentReader := ExpandConstant('{app}\current\chaptera-reader.exe');
    if not FileExists(CurrentReader) then
      RaiseException('Chaptera Reader current-layout payload is missing after install');

    LegacyReader := ExpandConstant('{app}\chaptera-reader.exe');
    if FileExists(LegacyReader) and (not DeleteFile(LegacyReader)) then
      RaiseException('Could not remove legacy root-level Chaptera Reader executable');

    LegacyReadme := ExpandConstant('{app}\README.md');
    if FileExists(LegacyReadme) and (not DeleteFile(LegacyReadme)) then
      RaiseException('Could not remove legacy root-level Chaptera Reader README');
  end;
end;
