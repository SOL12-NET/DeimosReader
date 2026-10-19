; Inno Setup Script for Deimos Reader (Windows x64)

#define MyAppName "Deimos Reader"
#define MyAppVersion "0.6 Beta"
#define MyAppNumericVersion "0.6.0.0"
#define MyAppPublisher "SOL12.NET"
#define MyAppURL "https://github.com/SOL12-NET/DeimosReader"
#define MyAppExeName "DeimosReader.exe"

[Setup]
AppId={{8E9D5C2B-4A61-4F38-9C1E-7A2D4F8B1012}
AppName={#MyAppName}
AppVersion={#MyAppVersion}
VersionInfoVersion={#MyAppNumericVersion}
AppPublisher={#MyAppPublisher}
AppPublisherURL={#MyAppURL}
AppSupportURL={#MyAppURL}
AppUpdatesURL={#MyAppURL}
LicenseFile=..\LICENSE
DefaultDirName={autopf}\{#MyAppName}
DefaultGroupName={#MyAppName}
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
OutputDir=..\target\installer
OutputBaseFilename=DeimosReader-Setup
SetupIconFile=..\assets\mars_icon.ico
UninstallDisplayIcon={app}\{#MyAppExeName}
UninstallDisplayName={#MyAppName} ({#MyAppVersion})
Compression=lzma2/ultra64
SolidCompression=yes
WizardStyle=modern
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"
Name: "associatepdf"; Description: "Associate .pdf files with {#MyAppName}"; GroupDescription: "File Associations:"

[Files]
Source: "..\target\release\deimos-reader.exe"; DestDir: "{app}"; DestName: "{#MyAppExeName}"; Flags: ignoreversion
Source: "..\pdfium.dll"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\assets\mars_icon.ico"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\LICENSE"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\{#MyAppName}"; Filename: "{app}\{#MyAppExeName}"; IconFilename: "{app}\mars_icon.ico"
Name: "{autodesktop}\{#MyAppName}"; Filename: "{app}\{#MyAppExeName}"; IconFilename: "{app}\mars_icon.ico"; Tasks: desktopicon

[Registry]
Root: HKA; Subkey: "Software\Classes\.pdf\OpenWithProgids"; ValueType: string; ValueName: "DeimosReader.PDF"; ValueData: ""; Flags: uninsdeletevalue; Tasks: associatepdf
Root: HKA; Subkey: "Software\Classes\DeimosReader.PDF"; ValueType: string; ValueName: ""; ValueData: "PDF Document"; Flags: uninsdeletekey; Tasks: associatepdf
Root: HKA; Subkey: "Software\Classes\DeimosReader.PDF\DefaultIcon"; ValueType: string; ValueName: ""; ValueData: "{app}\{#MyAppExeName},0"; Tasks: associatepdf
Root: HKA; Subkey: "Software\Classes\DeimosReader.PDF\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\{#MyAppExeName}"" ""%1"""; Tasks: associatepdf

[Run]
Filename: "{app}\{#MyAppExeName}"; Description: "{cm:LaunchProgram,{#StringChange(MyAppName, '&', '&&')}}"; Flags: nowait postinstall skipifsilent
