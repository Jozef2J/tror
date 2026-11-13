#define Version GetEnv("TROR_VERSION")

[Setup]
AppId={{8F3B6C1E-5C0A-4E4B-9A55-7A6B1C0D2E11}
AppName=Tror
AppVersion={#Version}
AppVerName=Tror
AppPublisher=Jozef2J
AppPublisherURL=https://github.com/Jozef2J/tror
DefaultDirName={autopf}\Tror
DefaultGroupName=Tror
DisableProgramGroupPage=yes
UninstallDisplayIcon={app}\tror.exe
OutputDir=..
OutputBaseFilename=tror-setup
SetupIconFile=..\assets\tror.ico
Compression=lzma2
SolidCompression=yes
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
CloseApplications=yes
RestartApplications=no
WizardStyle=modern

[Languages]
Name: "ru"; MessagesFile: "compiler:Languages\Russian.isl"
Name: "en"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; Flags: unchecked
Name: "menu"; Description: "Пункт «Открыть в Tror» в меню файлов и папок"

[Files]
Source: "..\target\release\tror.exe"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\Tror"; Filename: "{app}\tror.exe"
Name: "{autodesktop}\Tror"; Filename: "{app}\tror.exe"; Tasks: desktopicon

[Registry]
Root: HKA; Subkey: "Software\Classes\*\shell\Tror"; ValueType: string; ValueData: "Открыть в Tror"; Flags: uninsdeletekey; Tasks: menu
Root: HKA; Subkey: "Software\Classes\*\shell\Tror"; ValueType: string; ValueName: "Icon"; ValueData: "{app}\tror.exe"; Tasks: menu
Root: HKA; Subkey: "Software\Classes\*\shell\Tror\command"; ValueType: string; ValueData: """{app}\tror.exe"" ""%1"""; Tasks: menu
Root: HKA; Subkey: "Software\Classes\Directory\shell\Tror"; ValueType: string; ValueData: "Открыть в Tror"; Flags: uninsdeletekey; Tasks: menu
Root: HKA; Subkey: "Software\Classes\Directory\shell\Tror"; ValueType: string; ValueName: "Icon"; ValueData: "{app}\tror.exe"; Tasks: menu
Root: HKA; Subkey: "Software\Classes\Directory\shell\Tror\command"; ValueType: string; ValueData: """{app}\tror.exe"" ""%1"""; Tasks: menu

[Run]
Filename: "{app}\tror.exe"; Description: "{cm:LaunchProgram,Tror}"; Flags: nowait postinstall skipifsilent
Filename: "{app}\tror.exe"; Flags: nowait runasoriginaluser; Check: WizardSilent
