; Inno Setup script for 3D Review.
;
; Do not run this directly — invoke packaging\build-windows-installer.ps1, which builds
; the release exe, reads product.json (the canonical source of product identity)
; and passes the values below as /D defines. Defaults are provided so the script
; can also be opened standalone in the Inno Setup IDE for editing.
;
; Requires Inno Setup 6 (ISCC.exe). https://jrsoftware.org/isdl.php

#ifndef MyAppName
  #define MyAppName "3D Review"
#endif
#ifndef MyAppVersion
  #define MyAppVersion "0.0.0"
#endif
#ifndef MyAppPublisher
  #define MyAppPublisher "Chandan Singh"
#endif
#ifndef MyAppExe
  #define MyAppExe "3d-review.exe"
#endif
; The release file's stem — product.json's exeName, so the installer is named
; <exeName>-<version>.exe, the same scheme as the macOS .dmg.
#ifndef MyAppFileStem
  #define MyAppFileStem "3d-review"
#endif
#ifndef MyAppProgId
  #define MyAppProgId "3DReview.fbx"
#endif
#ifndef MyAppTypeName
  #define MyAppTypeName "FBX 3D Model"
#endif
#ifndef MyAppUrl
  #define MyAppUrl "https://github.com/psmyles/3d-review"
#endif

[Setup]
; A stable GUID keeps upgrades/uninstall tied to the same app across versions.
AppId={{B6F4B0E9-2E1B-4C3A-9D2F-3D5E7A1C0F42}
AppName={#MyAppName}
AppVersion={#MyAppVersion}
AppPublisher={#MyAppPublisher}
AppPublisherURL={#MyAppUrl}
AppSupportURL={#MyAppUrl}
DefaultDirName={autopf}\{#MyAppName}
DefaultGroupName={#MyAppName}
DisableProgramGroupPage=yes
; Let the user pick all-users (admin) vs just-me (no elevation) at install time.
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
OutputDir=..\dist
OutputBaseFilename={#MyAppFileStem}-{#MyAppVersion}
SetupIconFile=..\assets\icons\application-logo.ico
UninstallDisplayIcon={app}\{#MyAppExe}
UninstallDisplayName={#MyAppName}
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
; The "addtopath" task edits the user's PATH: this is what has Setup (and the
; uninstaller) tell open Explorer windows and terminals started from them to
; reload it, rather than leave the change unseen until the next sign-in.
ChangesEnvironment=yes

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "associate"; GroupDescription: "File associations:"; \
  Description: "Associate .fbx files with {#MyAppName}"
Name: "addtopath"; GroupDescription: "Command line:"; \
  Description: "Add {#MyAppName} to my PATH, so ""review-cli"" runs from any terminal"; \
  Flags: unchecked
Name: "desktopicon"; GroupDescription: "Additional icons:"; \
  Description: "Create a desktop shortcut"; Flags: unchecked

[Files]
; The exe carries almost everything: toolbar/window icons, the baked lighting
; maps, the message catalogs and every manual page's *text* are all
; include_bytes!-embedded.
Source: "..\target\release\{#MyAppExe}"; DestDir: "{app}"; Flags: ignoreversion
; The console tool (docs\cli.md): the Opt and Aud workspaces for scripts,
; pipelines and Launcher. Beside the viewer, so anything that finds the install
; through its Uninstall key's InstallLocation finds this too.
Source: "..\target\release\review-cli.exe"; DestDir: "{app}"; Flags: ignoreversion
; The manual's images are the one exception (invariant 12): embedding
; screenshots would put megabytes in a binary whose startup time is a feature.
; An install without them still shows every page — each image renders as its
; alt text — which is why this entry is allowed to find nothing.
Source: "..\docs\book\src\en\images\*"; DestDir: "{app}\docs\en\images"; Flags: ignoreversion recursesubdirs createallsubdirs skipifsourcedoesntexist
; --- license notices ---------------------------------------------------------
; The exe is statically linked, so it contains code from every project listed in
; docs\THIRD-PARTY-NOTICES.md. Their licenses require the notices to travel with
; the binary, so a user who only ever runs Setup.exe still receives them. Not
; optional and not behind a task: dropping these makes the installed copy
; non-compliant. build-windows-installer.ps1 fails before ISCC if one is missing.
Source: "..\LICENSE"; DestDir: "{app}"; DestName: "LICENSE.txt"; Flags: ignoreversion
Source: "..\docs\THIRD-PARTY-NOTICES.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\docs\CREDITS.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\licenses\*.txt"; DestDir: "{app}\licenses"; Flags: ignoreversion
; No external tools to ship: PSD is decoded in-process by the prebuilt psd_sdk FFI
; crate (review-psd), JPEG by zune, and the rest by the Rust `image` crate — the
; old bundled ImageMagick CLI is gone.

[Icons]
Name: "{group}\{#MyAppName}"; Filename: "{app}\{#MyAppExe}"
Name: "{group}\Uninstall {#MyAppName}"; Filename: "{uninstallexe}"
Name: "{autodesktop}\{#MyAppName}"; Filename: "{app}\{#MyAppExe}"; Tasks: desktopicon

[Registry]
; Register a ProgID and add it to the .fbx Open-With list instead of hijacking
; the user's existing default handler. HKA resolves to HKLM (all-users) or HKCU
; (just-me) to match the chosen install scope. uninsdeletekey removes these on
; uninstall.
Root: HKA; Subkey: "Software\Classes\{#MyAppProgId}"; ValueType: string; ValueName: ""; \
  ValueData: "{#MyAppTypeName}"; Flags: uninsdeletekey; Tasks: associate
Root: HKA; Subkey: "Software\Classes\{#MyAppProgId}\DefaultIcon"; ValueType: string; ValueName: ""; \
  ValueData: "{app}\{#MyAppExe},0"; Tasks: associate
Root: HKA; Subkey: "Software\Classes\{#MyAppProgId}\shell\open\command"; ValueType: string; ValueName: ""; \
  ValueData: """{app}\{#MyAppExe}"" ""%1"""; Tasks: associate
Root: HKA; Subkey: "Software\Classes\.fbx\OpenWithProgIds"; ValueType: string; ValueName: "{#MyAppProgId}"; \
  ValueData: ""; Flags: uninsdeletevalue; Tasks: associate

[Run]
Filename: "{app}\{#MyAppExe}"; Description: "Launch {#MyAppName}"; \
  Flags: nowait postinstall skipifsilent

[Code]
// The PATH entry behind the "addtopath" task, so `review-cli` runs from any
// terminal. Only the per-user PATH is touched, whichever install scope was
// chosen: an all-users install would need the machine variable, and appending to
// that from a setup that may be running unelevated is not safe. Launcher and other
// tools need none of it - they find review-cli.exe beside the viewer through the
// Uninstall key. The entry is removed on uninstall. (The same code as Bite's
// installer.)

const
  EnvironmentKey = 'Environment';

function PathDirs(const Path: string): TArrayOfString;
var
  Remaining, Item: string;
  Count, Position: Integer;
begin
  SetArrayLength(Result, 0);
  Remaining := Path;
  Count := 0;
  while Remaining <> '' do
  begin
    Position := Pos(';', Remaining);
    if Position = 0 then
    begin
      Item := Remaining;
      Remaining := '';
    end
    else
    begin
      Item := Copy(Remaining, 1, Position - 1);
      Remaining := Copy(Remaining, Position + 1, Length(Remaining) - Position);
    end;
    Item := Trim(Item);
    if Item <> '' then
    begin
      SetArrayLength(Result, Count + 1);
      Result[Count] := Item;
      Count := Count + 1;
    end;
  end;
end;

// True when Directory is already listed in Path. Compared case-insensitively and
// without any trailing slash, so a second install does not add a duplicate entry.
function PathContains(const Path, Directory: string): Boolean;
var
  Dirs: TArrayOfString;
  Wanted, Item: string;
  I: Integer;
begin
  Result := False;
  Wanted := Lowercase(RemoveBackslashUnlessRoot(Directory));
  Dirs := PathDirs(Path);
  for I := 0 to GetArrayLength(Dirs) - 1 do
  begin
    Item := Lowercase(RemoveBackslashUnlessRoot(Dirs[I]));
    if Item = Wanted then
    begin
      Result := True;
      Exit;
    end;
  end;
end;

procedure AddToUserPath(const Directory: string);
var
  Path: string;
begin
  if not RegQueryStringValue(HKCU, EnvironmentKey, 'Path', Path) then
    Path := '';
  if PathContains(Path, Directory) then
    Exit;
  if (Path <> '') and (Copy(Path, Length(Path), 1) <> ';') then
    Path := Path + ';';
  // REG_EXPAND_SZ: an existing PATH commonly holds %USERPROFILE% and friends, and
  // rewriting it as a plain string would freeze those at today's values.
  RegWriteExpandStringValue(HKCU, EnvironmentKey, 'Path', Path + Directory);
end;

procedure RemoveFromUserPath(const Directory: string);
var
  Path, Rebuilt, Wanted, Item: string;
  Dirs: TArrayOfString;
  I: Integer;
begin
  if not RegQueryStringValue(HKCU, EnvironmentKey, 'Path', Path) then
    Exit;
  Wanted := Lowercase(RemoveBackslashUnlessRoot(Directory));
  Dirs := PathDirs(Path);
  Rebuilt := '';
  for I := 0 to GetArrayLength(Dirs) - 1 do
  begin
    Item := Dirs[I];
    if Lowercase(RemoveBackslashUnlessRoot(Item)) <> Wanted then
    begin
      if Rebuilt <> '' then
        Rebuilt := Rebuilt + ';';
      Rebuilt := Rebuilt + Item;
    end;
  end;
  if Rebuilt = Path then
    Exit;
  if Rebuilt = '' then
    RegDeleteValue(HKCU, EnvironmentKey, 'Path')
  else
    RegWriteExpandStringValue(HKCU, EnvironmentKey, 'Path', Rebuilt);
end;

procedure CurStepChanged(CurStep: TSetupStep);
begin
  // ssPostInstall, so the install directory exists and the task selection is final.
  if (CurStep = ssPostInstall) and WizardIsTaskSelected('addtopath') then
    AddToUserPath(ExpandConstant('{app}'));
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
begin
  if CurUninstallStep = usUninstall then
    RemoveFromUserPath(ExpandConstant('{app}'));
end;
