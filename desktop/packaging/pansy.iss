; pansy's installer for Windows, compiled by installers.sh with Inno Setup 6:
;
;   ISCC /DVersion=0.2.0 /DNumbers=0.2.0.7 /DSource=<build.sh's pansy\>
;        /DOutput=<dir> /DOutputName=<name> pansy.iss
;
; It puts what build.sh staged in %LOCALAPPDATA%\Programs\Pansy, for this
; user alone and without asking for an administrator: the window's updater
; replaces every entry of that directory (nsh-updater swaps them in place), so
; the directory must be the user's, and the uninstaller lives beside it, not
; in it. Uninstalling removes the directory whole, since what an update put
; there is not in Inno's record; pansy's own files (~/.config/pansy) stay.
; Saved as UTF-8 with a byte order mark, which is how Inno reads it so.

#ifndef Version
  #error Version is not defined: run packaging/installers.sh
#endif

[Setup]
; the same for every version: what makes a newer installer an upgrade
AppId={{583A6A30-6EBA-4BB2-A3DF-3BCEA4F9D443}
AppName=Pansy
AppVersion={#Version}
AppVerName=Pansy {#Version}
AppPublisher=pansy
AppPublisherURL=https://github.com/pansy-ai/pansy
AppSupportURL=https://github.com/pansy-ai/pansy/issues
AppUpdatesURL=https://releases.pansyai.dev/index.html
VersionInfoVersion={#Numbers}
DefaultDirName={localappdata}\Programs\Pansy
DisableDirPage=yes
DisableProgramGroupPage=yes
UninstallFilesDir={localappdata}\Programs\Pansy Uninstall
UninstallDisplayIcon={app}\pansy.exe
UninstallDisplayName=Pansy
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
SetupIconFile=pansy.ico
WizardStyle=modern
Compression=lzma2/max
SolidCompression=yes
ChangesEnvironment=yes
CloseApplications=yes
OutputDir={#Output}
OutputBaseFilename={#OutputName}

[Languages]
Name: "en"; MessagesFile: "compiler:Default.isl"
#if FileExists(CompilerPath + "Languages\ChineseSimplified.isl")
Name: "zh"; MessagesFile: "compiler:Languages\ChineseSimplified.isl"
#endif

[CustomMessages]
en.AddToPath=Add the pansy command to PATH
#if FileExists(CompilerPath + "Languages\ChineseSimplified.isl")
zh.AddToPath=将 pansy 命令加入 PATH
#endif

[Tasks]
Name: "path"; Description: "{cm:AddToPath}"
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "{#Source}\*"; DestDir: "{app}"; Flags: ignoreversion recursesubdirs createallsubdirs

[Icons]
Name: "{autoprograms}\Pansy"; Filename: "{app}\pansy.exe"
Name: "{autodesktop}\Pansy"; Filename: "{app}\pansy.exe"; Tasks: desktopicon

[Run]
Filename: "{app}\pansy.exe"; Description: "{cm:LaunchProgram,Pansy}"; Flags: nowait postinstall skipifsilent

[UninstallDelete]
Type: filesandordirs; Name: "{app}"

[Code]
const
  EnvironmentKey = 'Environment';

{ Whether Dir is one of the ;-separated Paths, as Windows compares them }
function PathHolds(Paths, Dir: string): Boolean;
begin
  Result := Pos(';' + Uppercase(Dir) + ';', ';' + Uppercase(Paths) + ';') > 0;
end;

procedure AddToPath(Dir: string);
var
  Paths: string;
begin
  if not RegQueryStringValue(HKCU, EnvironmentKey, 'Path', Paths) then
    Paths := '';
  if PathHolds(Paths, Dir) then
    exit;
  if (Paths <> '') and (Paths[Length(Paths)] <> ';') then
    Paths := Paths + ';';
  RegWriteExpandStringValue(HKCU, EnvironmentKey, 'Path', Paths + Dir);
end;

procedure RemoveFromPath(Dir: string);
var
  Paths: string;
  At: Integer;
begin
  if not RegQueryStringValue(HKCU, EnvironmentKey, 'Path', Paths) then
    exit;
  Paths := ';' + Paths + ';';
  At := Pos(';' + Uppercase(Dir) + ';', Uppercase(Paths));
  if At = 0 then
    exit;
  Delete(Paths, At, Length(Dir) + 1);
  RegWriteExpandStringValue(HKCU, EnvironmentKey, 'Path', Copy(Paths, 2, Length(Paths) - 2));
end;

procedure CurStepChanged(CurStep: TSetupStep);
begin
  if (CurStep = ssPostInstall) and WizardIsTaskSelected('path') then
    AddToPath(ExpandConstant('{app}'));
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
begin
  if CurUninstallStep = usPostUninstall then
    RemoveFromPath(ExpandConstant('{app}'));
end;
