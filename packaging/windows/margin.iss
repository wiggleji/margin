; Margin installer (Inno Setup 6). Built by scripts/package-windows.ps1:
;   ISCC.exe /DAppVersion=0.2.1 packaging\windows\margin.iss
; Per-user by default (no administrator needed); "Install for all users"
; is offered when run as administrator.

#ifndef AppVersion
  #define AppVersion "0.0.0"
#endif

[Setup]
AppId={{12A9CD6B-EE14-466A-8BD7-22863ACAF854}
AppName=Margin
AppVersion={#AppVersion}
AppVerName=Margin {#AppVersion}
AppPublisher=Margin contributors
AppPublisherURL=https://github.com/wiggleji/margin
AppSupportURL=https://github.com/wiggleji/margin/issues
AppUpdatesURL=https://github.com/wiggleji/margin/releases
DefaultDirName={autopf}\Margin
DefaultGroupName=Margin
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
LicenseFile=..\..\LICENSE
SetupIconFile=margin.ico
UninstallDisplayIcon={app}\margin.exe
OutputDir=..\..\dist
OutputBaseFilename=Margin-{#AppVersion}-windows-x64-setup
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
ChangesEnvironment=yes
CloseApplications=yes

[Tasks]
Name: "addtopath"; Description: "Add the margin command to PATH (type margin . in a terminal)"
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; Flags: unchecked

[Files]
Source: "..\..\target\release\margin.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\..\target\release\margin-mcp.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\..\LICENSE"; DestDir: "{app}"; DestName: "LICENSE.txt"
Source: "..\..\THIRD_PARTY_NOTICES.md"; DestDir: "{app}"

[Icons]
Name: "{autoprograms}\Margin"; Filename: "{app}\margin.exe"
Name: "{autodesktop}\Margin"; Filename: "{app}\margin.exe"; Tasks: desktopicon

[Run]
Filename: "{app}\margin.exe"; Description: "{cm:LaunchProgram,Margin}"; Flags: nowait postinstall skipifsilent

[Code]
// The user's PATH (HKCU\Environment), so "margin" works in new terminals.
const EnvKey = 'Environment';

function PathHasDir(Path, Dir: string): Boolean;
begin
  Result := Pos(';' + Uppercase(Dir) + ';', ';' + Uppercase(Path) + ';') > 0;
end;

procedure CurStepChanged(CurStep: TSetupStep);
var
  Path, Dir: string;
begin
  if (CurStep = ssPostInstall) and WizardIsTaskSelected('addtopath') then
  begin
    Dir := ExpandConstant('{app}');
    if not RegQueryStringValue(HKEY_CURRENT_USER, EnvKey, 'Path', Path) then Path := '';
    if not PathHasDir(Path, Dir) then
    begin
      if (Path <> '') and (Copy(Path, Length(Path), 1) <> ';') then Path := Path + ';';
      RegWriteExpandStringValue(HKEY_CURRENT_USER, EnvKey, 'Path', Path + Dir);
    end;
  end;
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
var
  Path, Dir: string;
  P: Integer;
begin
  if CurUninstallStep = usPostUninstall then
  begin
    Dir := ExpandConstant('{app}');
    if RegQueryStringValue(HKEY_CURRENT_USER, EnvKey, 'Path', Path) then
    begin
      Path := ';' + Path + ';';
      P := Pos(';' + Uppercase(Dir) + ';', Uppercase(Path));
      if P > 0 then
      begin
        Delete(Path, P, Length(Dir) + 1);
        Path := Copy(Path, 2, Length(Path) - 2);
        RegWriteExpandStringValue(HKEY_CURRENT_USER, EnvKey, 'Path', Path);
      end;
    end;
  end;
end;
