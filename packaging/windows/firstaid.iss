; ─────────────────────────────────────────────────────────────────────────────
; FirstAid Doctor — Inno Setup 6 installer script
;
; Produces a single self-extracting .exe installer without requiring WiX.
; Inno Setup is simpler than WiX and requires no XML namespaces or GUIDs.
;
; Prerequisites:
;   Install Inno Setup 6 from https://jrsoftware.org/isdl.php
;
; Build:
;   ISCC.exe packaging\windows\firstaid.iss
;   (or use scripts\package-windows.ps1 which calls ISCC automatically)
;
; Output: dist\windows\firstaid-<version>-setup.exe
; ─────────────────────────────────────────────────────────────────────────────

#define AppName      "FirstAid Doctor"
#define AppVersion   "0.1.0"
#define AppPublisher "FirstAid Doctor Contributors"
#define AppURL       "https://github.com/harshilshah2501/m-firstaid-doctor"
#define AppExeName   "firstaid.exe"
#define ServiceName  "firstaid"
#define BinDir       "..\..\target\x86_64-pc-windows-msvc\release"

[Setup]
; NOTE: The AppId value uniquely identifies this application.
; Do not change it once you ship — this GUID ties upgrades together.
AppId={{E3A7C1D2-8F4B-4E9A-B2C5-6D0F3A1E7B94}
AppName={#AppName}
AppVersion={#AppVersion}
AppVerName={#AppName} {#AppVersion}
AppPublisher={#AppPublisher}
AppPublisherURL={#AppURL}
AppSupportURL={#AppURL}
AppUpdatesURL={#AppURL}

; Per-machine install (requires admin)
PrivilegesRequired=admin
PrivilegesRequiredOverridesAllowed=dialog

DefaultDirName={autopf}\{#AppName}
DefaultGroupName={#AppName}
DisableProgramGroupPage=yes

; Silent install possible: setup.exe /SILENT
; Output filename
OutputDir=..\..\dist\windows
OutputBaseFilename=firstaid-{#AppVersion}-setup
Compression=lzma2/ultra64
SolidCompression=yes

; Require Windows 10 or later
MinVersion=10.0.17763

; Signing (optional — set these env vars in CI):
; SignTool=signtool sign /fd SHA256 /tr http://timestamp.digicert.com /td SHA256 /f "$ENV_CERT_PATH" /p "$ENV_CERT_PASS" $f
; SignedUninstaller=yes

WizardStyle=modern
SetupIconFile=
UninstallDisplayIcon={app}\{#AppExeName}

; Restart if any of our files are locked (service running during upgrade)
CloseApplications=yes
RestartApplications=no

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "startmenu";    Description: "Create a Start Menu shortcut"; GroupDescription: "Additional icons:"
Name: "addtopath";    Description: "Add {app} to system PATH";     GroupDescription: "Additional options:"; Flags: checkedonce

[Dirs]
Name: "{commonappdata}\{#AppName}";       Permissions: system-full users-readexec
Name: "{commonappdata}\{#AppName}\logs";  Permissions: system-full users-readexec

[Files]
Source: "{#BinDir}\{#AppExeName}";  DestDir: "{app}";  Flags: ignoreversion
; Bundle ORT DLL if present alongside the binary
Source: "{#BinDir}\onnxruntime.dll"; DestDir: "{app}"; Flags: ignoreversion skipifsourcedoesntexist

[Icons]
Name: "{group}\{#AppName} Shell"; \
      Filename: "{sys}\cmd.exe"; \
      Parameters: "/k firstaid --help"; \
      WorkingDir: "{app}"; \
      Tasks: startmenu
Name: "{group}\Uninstall {#AppName}"; \
      Filename: "{uninstallexe}"; \
      Tasks: startmenu

[Registry]
; Version stamp
Root: HKLM; Subkey: "SOFTWARE\FirstAidDoctor"; \
     ValueType: string; ValueName: "Version";     ValueData: "{#AppVersion}"; Flags: createvalueifdoesntexist uninsdeletekey
Root: HKLM; Subkey: "SOFTWARE\FirstAidDoctor"; \
     ValueType: string; ValueName: "InstallPath"; ValueData: "{app}";          Flags: createvalueifdoesntexist
Root: HKLM; Subkey: "SOFTWARE\FirstAidDoctor"; \
     ValueType: string; ValueName: "DataPath";    ValueData: "{commonappdata}\{#AppName}"; Flags: createvalueifdoesntexist

; Add to PATH
Root: HKLM; Subkey: "SYSTEM\CurrentControlSet\Control\Session Manager\Environment"; \
     ValueType: expandsz; ValueName: "Path"; \
     ValueData: "{olddata};{app}"; \
     Check: NeedsAddToPath(ExpandConstant('{app}')); \
     Tasks: addtopath

[Code]
// ── Windows service: binPath must quote the .exe when installed under "Program Files"
function GetScCreateParams(Param: string): string;
var
  AppPath, Svc, Exe: string;
begin
  AppPath := ExpandConstant('{app}');
  Svc := ExpandConstant('{#ServiceName}');
  Exe := ExpandConstant('{#AppExeName}');
  Result := 'create "' + Svc + '" binPath= "\"' + AppPath + '\' + Exe + '\" service" DisplayName= "FirstAid Doctor Agent" start= auto obj= LocalSystem';
end;

// ── PATH helper ───────────────────────────────────────────────────────────────
function NeedsAddToPath(Dir: string): boolean;
var
  OldPath: string;
begin
  if not RegQueryStringValue(
      HKLM,
      'SYSTEM\CurrentControlSet\Control\Session Manager\Environment',
      'Path', OldPath)
  then begin
    Result := True;
    exit;
  end;
  Result := Pos(';' + Uppercase(Dir) + ';',
                ';' + Uppercase(OldPath) + ';') = 0;
end;

// Remove our dir from PATH on uninstall
procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
var
  OldPath, NewPath, Dir: string;
  P: Integer;
begin
  if CurUninstallStep = usPostUninstall then begin
    Dir := ExpandConstant('{app}');
    if RegQueryStringValue(
        HKLM,
        'SYSTEM\CurrentControlSet\Control\Session Manager\Environment',
        'Path', OldPath)
    then begin
      NewPath := OldPath;
      P := Pos(';' + Uppercase(Dir), ';' + Uppercase(NewPath));
      if P > 0 then begin
        Delete(NewPath, P - 1, Length(Dir) + 1);
        RegWriteStringValue(
            HKLM,
            'SYSTEM\CurrentControlSet\Control\Session Manager\Environment',
            'Path', NewPath);
      end;
    end;
  end;
end;

[Run]
; Install and start the Windows service after files are copied (see GetScCreateParams for quoted binPath)
Filename: "{sys}\sc.exe"; \
  Parameters: "{code:GetScCreateParams}"; \
  Flags: runhidden; StatusMsg: "Installing FirstAid Doctor service…"

Filename: "{sys}\sc.exe"; \
  Parameters: "description ""{#ServiceName}"" ""Local-first endpoint health diagnostics and remediation"""; \
  Flags: runhidden

Filename: "{sys}\sc.exe"; \
  Parameters: "start ""{#ServiceName}"""; \
  Flags: runhidden; StatusMsg: "Starting FirstAid Doctor service…"

; Optionally open a shell to confirm it's running (skippable)
Filename: "{sys}\cmd.exe"; \
  Parameters: "/k firstaid --help"; \
  Description: "Open FirstAid Doctor shell"; \
  Flags: postinstall shellexec skipifsilent unchecked

[UninstallRun]
; Stop and remove the service before files are deleted
Filename: "{sys}\sc.exe"; Parameters: "stop ""{#ServiceName}""";   Flags: runhidden; RunOnceId: "StopService"
Filename: "{sys}\sc.exe"; Parameters: "delete ""{#ServiceName}"""; Flags: runhidden; RunOnceId: "DeleteService"
