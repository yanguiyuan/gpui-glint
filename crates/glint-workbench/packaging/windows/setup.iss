#ifndef AppVersion
  #error AppVersion must be supplied by build-windows.ps1
#endif
#ifndef StageDir
  #error StageDir must be supplied by build-windows.ps1
#endif
#ifndef OutputDir
  #error OutputDir must be supplied by build-windows.ps1
#endif

[Setup]
AppId={{7B7931DA-05B8-4E6B-9FD4-F33D82DD4B23}
AppName=Glint Workbench
AppVersion={#AppVersion}
AppPublisher=Yanguiyuan
AppPublisherURL=https://github.com/yanguiyuan/gpui-glint
DefaultDirName={localappdata}\Programs\Glint Workbench
DefaultGroupName=Glint Workbench
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0
DisableProgramGroupPage=yes
OutputDir={#OutputDir}
OutputBaseFilename=Glint-Workbench-{#AppVersion}-windows-x64-setup
SetupIconFile={#StageDir}\glint-workbench-icon.ico
UninstallDisplayIcon={app}\glint-workbench.exe
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
CloseApplications=yes
RestartApplications=no

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Files]
Source: "{#StageDir}\glint-workbench.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#StageDir}\MicrosoftEdgeWebview2Setup.exe"; DestDir: "{tmp}"; Flags: deleteafterinstall; Check: NeedsWebView2

[Icons]
Name: "{userdesktop}\Glint Workbench"; Filename: "{app}\glint-workbench.exe"; WorkingDir: "{app}"; IconFilename: "{app}\glint-workbench.exe"; AppUserModelID: "dev.glint.workbench"
Name: "{userprograms}\Glint Workbench\Glint Workbench"; Filename: "{app}\glint-workbench.exe"; WorkingDir: "{app}"; IconFilename: "{app}\glint-workbench.exe"; AppUserModelID: "dev.glint.workbench"

[Run]
Filename: "{app}\glint-workbench.exe"; Description: "Launch Glint Workbench"; Flags: nowait postinstall skipifsilent

[Code]
function HasWebView2(Root: Integer; Key: String): Boolean;
var
  Version: String;
begin
  Result := RegQueryStringValue(Root, Key, 'pv', Version) and (Version <> '') and (Version <> '0.0.0.0');
end;

function NeedsWebView2: Boolean;
var
  Key: String;
begin
  Key := 'SOFTWARE\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}';
  Result := not (HasWebView2(HKLM32, Key) or HasWebView2(HKLM64, Key) or HasWebView2(HKCU, Key));
end;

function PrepareToInstall(var NeedsRestart: Boolean): String;
var
  ExitCode: Integer;
begin
  Result := '';
  if not NeedsWebView2 then
    Exit;
  ExtractTemporaryFile('MicrosoftEdgeWebview2Setup.exe');
  if not Exec(ExpandConstant('{tmp}\MicrosoftEdgeWebview2Setup.exe'), '/silent /install', '', SW_HIDE, ewWaitUntilTerminated, ExitCode) then
    Result := 'Unable to start WebView2 installation. Please install Microsoft Edge WebView2 Runtime and retry.'
  else if NeedsWebView2 then
    Result := 'WebView2 Runtime could not be installed. Check your internet connection and retry.';
end;
