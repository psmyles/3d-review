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
OutputBaseFilename=3D-Review-Setup-{#MyAppVersion}
SetupIconFile=..\assets\icons\application-logo.ico
UninstallDisplayIcon={app}\{#MyAppExe}
UninstallDisplayName={#MyAppName}
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "associate"; GroupDescription: "File associations:"; \
  Description: "Associate .fbx files with {#MyAppName}"
Name: "desktopicon"; GroupDescription: "Additional icons:"; \
  Description: "Create a desktop shortcut"; Flags: unchecked

[Files]
; The exe carries almost everything: toolbar/window icons, the baked lighting
; maps, the message catalogs and every manual page's *text* are all
; include_bytes!-embedded.
Source: "..\target\release\{#MyAppExe}"; DestDir: "{app}"; Flags: ignoreversion
; The manual's images are the one exception (invariant 12): embedding
; screenshots would put megabytes in a binary whose startup time is a feature.
; An install without them still shows every page — each image renders as its
; alt text — which is why this entry is allowed to find nothing.
Source: "..\docs\book\src\en\images\*"; DestDir: "{app}\docs\en\images"; Flags: ignoreversion recursesubdirs createallsubdirs skipifsourcedoesntexist
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
