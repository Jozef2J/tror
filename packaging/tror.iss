#define Version GetEnv("TROR_VERSION")

[Setup]
AppId={{8F3B6C1E-5C0A-4E4B-9A55-7A6B1C0D2E11}
AppName=TrorPad
AppVersion={#Version}
AppVerName=TrorPad
AppPublisher=Jozef2J
AppPublisherURL=https://github.com/Jozef2J/tror
DefaultDirName={autopf}\TrorPad
DefaultGroupName=TrorPad
DisableProgramGroupPage=yes
UninstallDisplayIcon={app}\trorpad.exe
OutputDir=..
OutputBaseFilename=trorpad-setup
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
Name: "menu"; Description: "Пункт «Открыть в TrorPad» в меню файлов и папок"

[InstallDelete]
Type: files; Name: "{app}\tror.exe"
Type: files; Name: "{app}\tror.old"
Type: files; Name: "{autoprograms}\Tror.lnk"
Type: files; Name: "{autodesktop}\Tror.lnk"

[Files]
Source: "..\target\release\trorpad.exe"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\TrorPad"; Filename: "{app}\trorpad.exe"
Name: "{autodesktop}\TrorPad"; Filename: "{app}\trorpad.exe"; Tasks: desktopicon

[Registry]
Root: HKA; Subkey: "Software\Classes\*\shell\Tror"; Flags: deletekey dontcreatekey
Root: HKA; Subkey: "Software\Classes\Directory\shell\Tror"; Flags: deletekey dontcreatekey
Root: HKA; Subkey: "Software\Classes\*\shell\TrorPad"; ValueType: string; ValueData: "Открыть в TrorPad"; Flags: uninsdeletekey; Tasks: menu
Root: HKA; Subkey: "Software\Classes\*\shell\TrorPad"; ValueType: string; ValueName: "Icon"; ValueData: "{app}\trorpad.exe"; Tasks: menu
Root: HKA; Subkey: "Software\Classes\*\shell\TrorPad\command"; ValueType: string; ValueData: """{app}\trorpad.exe"" ""%1"""; Tasks: menu
Root: HKA; Subkey: "Software\Classes\Directory\shell\TrorPad"; ValueType: string; ValueData: "Открыть в TrorPad"; Flags: uninsdeletekey; Tasks: menu
Root: HKA; Subkey: "Software\Classes\Directory\shell\TrorPad"; ValueType: string; ValueName: "Icon"; ValueData: "{app}\trorpad.exe"; Tasks: menu
Root: HKA; Subkey: "Software\Classes\Directory\shell\TrorPad\command"; ValueType: string; ValueData: """{app}\trorpad.exe"" ""%1"""; Tasks: menu

[Run]
Filename: "{app}\trorpad.exe"; Description: "{cm:LaunchProgram,TrorPad}"; Flags: nowait postinstall skipifsilent
Filename: "{app}\trorpad.exe"; Flags: nowait runasoriginaluser; Check: WizardSilent
