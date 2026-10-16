; Maintainer input only: compile using release/build-installer.ps1.
; End users run the resulting Setup.exe; no compilers or runtimes are installed.
#if VER != 0x06070300
  #error Use the qualified Inno Setup 6.7.3 compiler.
#endif
#ifndef PayloadDir
  #error PayloadDir is required
#endif
#ifndef AppVersion
  #error AppVersion is required
#endif
#ifndef Architecture
  #error Architecture is required
#endif
#ifndef OutputDir
  #error OutputDir is required
#endif

[Setup]
AppId=Reframe.OptChat.Desktop
AppName=OptChat
AppVersion={#AppVersion}
AppPublisher=OptChat
AppPublisherURL=https://github.com/fabihatasneem/optchat-client
AppUpdatesURL=https://github.com/fabihatasneem/optchat-client/releases
VersionInfoVersion={#AppVersion}
DefaultDirName={localappdata}\Programs\OptChat
DefaultGroupName=OptChat
DisableDirPage=yes
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
#if Architecture == "x64"
ArchitecturesAllowed=x64compatible and not arm64
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0.19045
#elif Architecture == "arm64"
ArchitecturesAllowed=arm64
ArchitecturesInstallIn64BitMode=arm64
MinVersion=10.0.22000
#else
  #error Unsupported architecture
#endif
OutputDir={#OutputDir}
OutputBaseFilename=OptChat-{#AppVersion}-windows-{#Architecture}-Setup
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
SetupIconFile=assets/OptChat.ico
SetupLogging=yes
CloseApplications=yes
RestartApplications=no
UninstallDisplayIcon={app}\optchat.exe
UninstallDisplayName=OptChat
#ifdef SignInstaller
SignTool=OptChatSign
SignedUninstaller=yes
#endif

[Tasks]
Name: "desktopicon"; Description: "Create a desktop shortcut"; GroupDescription: "Shortcuts:"; Flags: unchecked

[Files]
Source: "{#PayloadDir}\optchat.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#PayloadDir}\build-info.json"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#PayloadDir}\notices\*"; DestDir: "{app}\notices"; Flags: ignoreversion recursesubdirs createallsubdirs

[InstallDelete]
; Remove obsolete diagnostics from earlier installations, never user data.
Type: files; Name: "{app}\imports.txt"
Type: files; Name: "{app}\INSTALL.txt"

[Icons]
Name: "{group}\OptChat"; Filename: "{app}\optchat.exe"
Name: "{autodesktop}\OptChat"; Filename: "{app}\optchat.exe"; Tasks: desktopicon

[Run]
Filename: "{app}\optchat.exe"; Description: "Launch OptChat"; Flags: nowait postinstall skipifsilent

; Deliberately no AppData or Credential Manager deletion during uninstall.

[Code]
function InitializeSetup(): Boolean;
var
  InstalledVersion: String;
  Installed, Candidate: Int64;
begin
  Result := True;
  if RegQueryStringValue(HKCU,
    'Software\Microsoft\Windows\CurrentVersion\Uninstall\Reframe.OptChat.Desktop_is1',
    'DisplayVersion', InstalledVersion) then
  begin
    if not StrToVersion(InstalledVersion, Installed) then
    begin
      SuppressibleMsgBox('The installed OptChat version could not be read. Uninstall it from Windows Settings before continuing.', mbError, MB_OK, IDOK);
      Result := False;
    end
    else if StrToVersion('{#AppVersion}', Candidate) and
      (ComparePackedVersion(Installed, Candidate) > 0) then
    begin
      SuppressibleMsgBox('A newer version of OptChat is already installed. Download the latest installer from the OptChat releases page.', mbError, MB_OK, IDOK);
      Result := False;
    end;
  end;
end;
