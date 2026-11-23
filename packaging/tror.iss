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
PrivilegesRequired=admin
PrivilegesRequiredOverridesAllowed=dialog
UsePreviousPrivileges=yes
ChangesAssociations=yes
CloseApplications=yes
RestartApplications=no
WizardStyle=modern

[Languages]
Name: "en"; MessagesFile: "compiler:Default.isl"
Name: "ru"; MessagesFile: "compiler:Languages\Russian.isl"

[CustomMessages]
ru.MenuTask=Пункт «Открыть в TrorPad» в меню файлов и папок
en.MenuTask="Open in TrorPad" entry in the file and folder menu
ru.AssocTask=Открывать в TrorPad по умолчанию
en.AssocTask=Open with TrorPad by default
ru.AssocGroup=Ассоциации файлов:
en.AssocGroup=File associations:
ru.AssocText=Текст и журналы
en.AssocText=Text and logs
ru.AssocWeb=Веб
en.AssocWeb=Web
ru.AssocPhp=PHP и шаблоны
en.AssocPhp=PHP and templates
ru.AssocCode=Скрипты и данные
en.AssocCode=Scripts and data
ru.OpenIn=Открыть в TrorPad
en.OpenIn=Open in TrorPad
ru.FileType=Файл TrorPad
en.FileType=TrorPad file
ru.AskSettings=Удалить настройки TrorPad (тема, раскладка панелей, недавние файлы, сочетания клавиш)?
en.AskSettings=Delete TrorPad settings (theme, panel layout, recent files, shortcuts)?
ru.AskProfiles=Удалить профили подключений (хосты SFTP/FTP, базы данных, консоль) и сохранённые пароли?
en.AskProfiles=Delete connection profiles (SFTP/FTP hosts, databases, console) and saved passwords?
ru.AskData=Удалить остальные данные (история запросов, несохранённые правки, резервные копии, плагины)?
en.AskData=Delete other data (query history, unsaved edits, backups, plugins)?

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; Flags: unchecked
Name: "menu"; Description: "{cm:MenuTask}"
Name: "assoc"; Description: "{cm:AssocTask}"; GroupDescription: "{cm:AssocGroup}"; Flags: unchecked
Name: "assoc\text"; Description: "{cm:AssocText} (.txt, .log, .md, .ini, .cfg, .conf, .env)"
Name: "assoc\web"; Description: "{cm:AssocWeb} (.html, .htm, .css, .scss, .js, .ts, .json, .xml, .svg)"
Name: "assoc\php"; Description: "{cm:AssocPhp} (.php, .phtml, .tpl, .twig)"
Name: "assoc\code"; Description: "{cm:AssocCode} (.sh, .py, .sql, .yml, .yaml, .csv)"

[InstallDelete]
Type: files; Name: "{app}\tror.exe"
Type: files; Name: "{app}\tror.old"
Type: files; Name: "{autoprograms}\Tror.lnk"
Type: files; Name: "{autodesktop}\Tror.lnk"

[Files]
Source: "..\target\release\trorpad.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\LICENSE-MIT"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\LICENSE-APACHE"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\TrorPad"; Filename: "{app}\trorpad.exe"
Name: "{autodesktop}\TrorPad"; Filename: "{app}\trorpad.exe"; Tasks: desktopicon

[Registry]
Root: HKA; Subkey: "Software\Classes\*\shell\Tror"; Flags: deletekey dontcreatekey
Root: HKA; Subkey: "Software\Classes\Directory\shell\Tror"; Flags: deletekey dontcreatekey
Root: HKA; Subkey: "Software\Classes\*\shell\TrorPad"; ValueType: string; ValueData: "{cm:OpenIn}"; Flags: uninsdeletekey; Tasks: menu
Root: HKA; Subkey: "Software\Classes\*\shell\TrorPad"; ValueType: string; ValueName: "Icon"; ValueData: "{app}\trorpad.exe"; Tasks: menu
Root: HKA; Subkey: "Software\Classes\*\shell\TrorPad\command"; ValueType: string; ValueData: """{app}\trorpad.exe"" ""%1"""; Tasks: menu
Root: HKA; Subkey: "Software\Classes\Directory\shell\TrorPad"; ValueType: string; ValueData: "{cm:OpenIn}"; Flags: uninsdeletekey; Tasks: menu
Root: HKA; Subkey: "Software\Classes\Directory\shell\TrorPad"; ValueType: string; ValueName: "Icon"; ValueData: "{app}\trorpad.exe"; Tasks: menu
Root: HKA; Subkey: "Software\Classes\Directory\shell\TrorPad\command"; ValueType: string; ValueData: """{app}\trorpad.exe"" ""%1"""; Tasks: menu

Root: HKA; Subkey: "Software\Classes\TrorPad.File"; ValueType: string; ValueData: "{cm:FileType}"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Classes\TrorPad.File\DefaultIcon"; ValueType: string; ValueData: "{app}\trorpad.exe,0"
Root: HKA; Subkey: "Software\Classes\TrorPad.File\shell\open\command"; ValueType: string; ValueData: """{app}\trorpad.exe"" ""%1"""
Root: HKA; Subkey: "Software\Classes\Applications\trorpad.exe"; ValueType: string; ValueName: "FriendlyAppName"; ValueData: "TrorPad"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Classes\Applications\trorpad.exe\shell\open\command"; ValueType: string; ValueData: """{app}\trorpad.exe"" ""%1"""
Root: HKA; Subkey: "Software\Classes\.txt\OpenWithProgids"; ValueType: string; ValueName: "TrorPad.File"; ValueData: ""; Flags: uninsdeletevalue; Tasks: assoc\text
Root: HKA; Subkey: "Software\Classes\Applications\trorpad.exe\SupportedTypes"; ValueType: string; ValueName: ".txt"; ValueData: ""
Root: HKA; Subkey: "Software\Classes\.txt"; ValueType: string; ValueData: "TrorPad.File"; Flags: uninsdeletevalue; Tasks: assoc\text; Check: NoHandler('.txt')
Root: HKA; Subkey: "Software\Classes\.log\OpenWithProgids"; ValueType: string; ValueName: "TrorPad.File"; ValueData: ""; Flags: uninsdeletevalue; Tasks: assoc\text
Root: HKA; Subkey: "Software\Classes\Applications\trorpad.exe\SupportedTypes"; ValueType: string; ValueName: ".log"; ValueData: ""
Root: HKA; Subkey: "Software\Classes\.log"; ValueType: string; ValueData: "TrorPad.File"; Flags: uninsdeletevalue; Tasks: assoc\text; Check: NoHandler('.log')
Root: HKA; Subkey: "Software\Classes\.md\OpenWithProgids"; ValueType: string; ValueName: "TrorPad.File"; ValueData: ""; Flags: uninsdeletevalue; Tasks: assoc\text
Root: HKA; Subkey: "Software\Classes\Applications\trorpad.exe\SupportedTypes"; ValueType: string; ValueName: ".md"; ValueData: ""
Root: HKA; Subkey: "Software\Classes\.md"; ValueType: string; ValueData: "TrorPad.File"; Flags: uninsdeletevalue; Tasks: assoc\text; Check: NoHandler('.md')
Root: HKA; Subkey: "Software\Classes\.ini\OpenWithProgids"; ValueType: string; ValueName: "TrorPad.File"; ValueData: ""; Flags: uninsdeletevalue; Tasks: assoc\text
Root: HKA; Subkey: "Software\Classes\Applications\trorpad.exe\SupportedTypes"; ValueType: string; ValueName: ".ini"; ValueData: ""
Root: HKA; Subkey: "Software\Classes\.ini"; ValueType: string; ValueData: "TrorPad.File"; Flags: uninsdeletevalue; Tasks: assoc\text; Check: NoHandler('.ini')
Root: HKA; Subkey: "Software\Classes\.cfg\OpenWithProgids"; ValueType: string; ValueName: "TrorPad.File"; ValueData: ""; Flags: uninsdeletevalue; Tasks: assoc\text
Root: HKA; Subkey: "Software\Classes\Applications\trorpad.exe\SupportedTypes"; ValueType: string; ValueName: ".cfg"; ValueData: ""
Root: HKA; Subkey: "Software\Classes\.cfg"; ValueType: string; ValueData: "TrorPad.File"; Flags: uninsdeletevalue; Tasks: assoc\text; Check: NoHandler('.cfg')
Root: HKA; Subkey: "Software\Classes\.conf\OpenWithProgids"; ValueType: string; ValueName: "TrorPad.File"; ValueData: ""; Flags: uninsdeletevalue; Tasks: assoc\text
Root: HKA; Subkey: "Software\Classes\Applications\trorpad.exe\SupportedTypes"; ValueType: string; ValueName: ".conf"; ValueData: ""
Root: HKA; Subkey: "Software\Classes\.conf"; ValueType: string; ValueData: "TrorPad.File"; Flags: uninsdeletevalue; Tasks: assoc\text; Check: NoHandler('.conf')
Root: HKA; Subkey: "Software\Classes\.env\OpenWithProgids"; ValueType: string; ValueName: "TrorPad.File"; ValueData: ""; Flags: uninsdeletevalue; Tasks: assoc\text
Root: HKA; Subkey: "Software\Classes\Applications\trorpad.exe\SupportedTypes"; ValueType: string; ValueName: ".env"; ValueData: ""
Root: HKA; Subkey: "Software\Classes\.env"; ValueType: string; ValueData: "TrorPad.File"; Flags: uninsdeletevalue; Tasks: assoc\text; Check: NoHandler('.env')
Root: HKA; Subkey: "Software\Classes\.html\OpenWithProgids"; ValueType: string; ValueName: "TrorPad.File"; ValueData: ""; Flags: uninsdeletevalue; Tasks: assoc\web
Root: HKA; Subkey: "Software\Classes\Applications\trorpad.exe\SupportedTypes"; ValueType: string; ValueName: ".html"; ValueData: ""
Root: HKA; Subkey: "Software\Classes\.html"; ValueType: string; ValueData: "TrorPad.File"; Flags: uninsdeletevalue; Tasks: assoc\web; Check: NoHandler('.html')
Root: HKA; Subkey: "Software\Classes\.htm\OpenWithProgids"; ValueType: string; ValueName: "TrorPad.File"; ValueData: ""; Flags: uninsdeletevalue; Tasks: assoc\web
Root: HKA; Subkey: "Software\Classes\Applications\trorpad.exe\SupportedTypes"; ValueType: string; ValueName: ".htm"; ValueData: ""
Root: HKA; Subkey: "Software\Classes\.htm"; ValueType: string; ValueData: "TrorPad.File"; Flags: uninsdeletevalue; Tasks: assoc\web; Check: NoHandler('.htm')
Root: HKA; Subkey: "Software\Classes\.css\OpenWithProgids"; ValueType: string; ValueName: "TrorPad.File"; ValueData: ""; Flags: uninsdeletevalue; Tasks: assoc\web
Root: HKA; Subkey: "Software\Classes\Applications\trorpad.exe\SupportedTypes"; ValueType: string; ValueName: ".css"; ValueData: ""
Root: HKA; Subkey: "Software\Classes\.css"; ValueType: string; ValueData: "TrorPad.File"; Flags: uninsdeletevalue; Tasks: assoc\web; Check: NoHandler('.css')
Root: HKA; Subkey: "Software\Classes\.scss\OpenWithProgids"; ValueType: string; ValueName: "TrorPad.File"; ValueData: ""; Flags: uninsdeletevalue; Tasks: assoc\web
Root: HKA; Subkey: "Software\Classes\Applications\trorpad.exe\SupportedTypes"; ValueType: string; ValueName: ".scss"; ValueData: ""
Root: HKA; Subkey: "Software\Classes\.scss"; ValueType: string; ValueData: "TrorPad.File"; Flags: uninsdeletevalue; Tasks: assoc\web; Check: NoHandler('.scss')
Root: HKA; Subkey: "Software\Classes\.js\OpenWithProgids"; ValueType: string; ValueName: "TrorPad.File"; ValueData: ""; Flags: uninsdeletevalue; Tasks: assoc\web
Root: HKA; Subkey: "Software\Classes\Applications\trorpad.exe\SupportedTypes"; ValueType: string; ValueName: ".js"; ValueData: ""
Root: HKA; Subkey: "Software\Classes\.js"; ValueType: string; ValueData: "TrorPad.File"; Flags: uninsdeletevalue; Tasks: assoc\web; Check: NoHandler('.js')
Root: HKA; Subkey: "Software\Classes\.ts\OpenWithProgids"; ValueType: string; ValueName: "TrorPad.File"; ValueData: ""; Flags: uninsdeletevalue; Tasks: assoc\web
Root: HKA; Subkey: "Software\Classes\Applications\trorpad.exe\SupportedTypes"; ValueType: string; ValueName: ".ts"; ValueData: ""
Root: HKA; Subkey: "Software\Classes\.ts"; ValueType: string; ValueData: "TrorPad.File"; Flags: uninsdeletevalue; Tasks: assoc\web; Check: NoHandler('.ts')
Root: HKA; Subkey: "Software\Classes\.json\OpenWithProgids"; ValueType: string; ValueName: "TrorPad.File"; ValueData: ""; Flags: uninsdeletevalue; Tasks: assoc\web
Root: HKA; Subkey: "Software\Classes\Applications\trorpad.exe\SupportedTypes"; ValueType: string; ValueName: ".json"; ValueData: ""
Root: HKA; Subkey: "Software\Classes\.json"; ValueType: string; ValueData: "TrorPad.File"; Flags: uninsdeletevalue; Tasks: assoc\web; Check: NoHandler('.json')
Root: HKA; Subkey: "Software\Classes\.xml\OpenWithProgids"; ValueType: string; ValueName: "TrorPad.File"; ValueData: ""; Flags: uninsdeletevalue; Tasks: assoc\web
Root: HKA; Subkey: "Software\Classes\Applications\trorpad.exe\SupportedTypes"; ValueType: string; ValueName: ".xml"; ValueData: ""
Root: HKA; Subkey: "Software\Classes\.xml"; ValueType: string; ValueData: "TrorPad.File"; Flags: uninsdeletevalue; Tasks: assoc\web; Check: NoHandler('.xml')
Root: HKA; Subkey: "Software\Classes\.svg\OpenWithProgids"; ValueType: string; ValueName: "TrorPad.File"; ValueData: ""; Flags: uninsdeletevalue; Tasks: assoc\web
Root: HKA; Subkey: "Software\Classes\Applications\trorpad.exe\SupportedTypes"; ValueType: string; ValueName: ".svg"; ValueData: ""
Root: HKA; Subkey: "Software\Classes\.svg"; ValueType: string; ValueData: "TrorPad.File"; Flags: uninsdeletevalue; Tasks: assoc\web; Check: NoHandler('.svg')
Root: HKA; Subkey: "Software\Classes\.php\OpenWithProgids"; ValueType: string; ValueName: "TrorPad.File"; ValueData: ""; Flags: uninsdeletevalue; Tasks: assoc\php
Root: HKA; Subkey: "Software\Classes\Applications\trorpad.exe\SupportedTypes"; ValueType: string; ValueName: ".php"; ValueData: ""
Root: HKA; Subkey: "Software\Classes\.php"; ValueType: string; ValueData: "TrorPad.File"; Flags: uninsdeletevalue; Tasks: assoc\php; Check: NoHandler('.php')
Root: HKA; Subkey: "Software\Classes\.phtml\OpenWithProgids"; ValueType: string; ValueName: "TrorPad.File"; ValueData: ""; Flags: uninsdeletevalue; Tasks: assoc\php
Root: HKA; Subkey: "Software\Classes\Applications\trorpad.exe\SupportedTypes"; ValueType: string; ValueName: ".phtml"; ValueData: ""
Root: HKA; Subkey: "Software\Classes\.phtml"; ValueType: string; ValueData: "TrorPad.File"; Flags: uninsdeletevalue; Tasks: assoc\php; Check: NoHandler('.phtml')
Root: HKA; Subkey: "Software\Classes\.tpl\OpenWithProgids"; ValueType: string; ValueName: "TrorPad.File"; ValueData: ""; Flags: uninsdeletevalue; Tasks: assoc\php
Root: HKA; Subkey: "Software\Classes\Applications\trorpad.exe\SupportedTypes"; ValueType: string; ValueName: ".tpl"; ValueData: ""
Root: HKA; Subkey: "Software\Classes\.tpl"; ValueType: string; ValueData: "TrorPad.File"; Flags: uninsdeletevalue; Tasks: assoc\php; Check: NoHandler('.tpl')
Root: HKA; Subkey: "Software\Classes\.twig\OpenWithProgids"; ValueType: string; ValueName: "TrorPad.File"; ValueData: ""; Flags: uninsdeletevalue; Tasks: assoc\php
Root: HKA; Subkey: "Software\Classes\Applications\trorpad.exe\SupportedTypes"; ValueType: string; ValueName: ".twig"; ValueData: ""
Root: HKA; Subkey: "Software\Classes\.twig"; ValueType: string; ValueData: "TrorPad.File"; Flags: uninsdeletevalue; Tasks: assoc\php; Check: NoHandler('.twig')
Root: HKA; Subkey: "Software\Classes\.sh\OpenWithProgids"; ValueType: string; ValueName: "TrorPad.File"; ValueData: ""; Flags: uninsdeletevalue; Tasks: assoc\code
Root: HKA; Subkey: "Software\Classes\Applications\trorpad.exe\SupportedTypes"; ValueType: string; ValueName: ".sh"; ValueData: ""
Root: HKA; Subkey: "Software\Classes\.sh"; ValueType: string; ValueData: "TrorPad.File"; Flags: uninsdeletevalue; Tasks: assoc\code; Check: NoHandler('.sh')
Root: HKA; Subkey: "Software\Classes\.py\OpenWithProgids"; ValueType: string; ValueName: "TrorPad.File"; ValueData: ""; Flags: uninsdeletevalue; Tasks: assoc\code
Root: HKA; Subkey: "Software\Classes\Applications\trorpad.exe\SupportedTypes"; ValueType: string; ValueName: ".py"; ValueData: ""
Root: HKA; Subkey: "Software\Classes\.py"; ValueType: string; ValueData: "TrorPad.File"; Flags: uninsdeletevalue; Tasks: assoc\code; Check: NoHandler('.py')
Root: HKA; Subkey: "Software\Classes\.sql\OpenWithProgids"; ValueType: string; ValueName: "TrorPad.File"; ValueData: ""; Flags: uninsdeletevalue; Tasks: assoc\code
Root: HKA; Subkey: "Software\Classes\Applications\trorpad.exe\SupportedTypes"; ValueType: string; ValueName: ".sql"; ValueData: ""
Root: HKA; Subkey: "Software\Classes\.sql"; ValueType: string; ValueData: "TrorPad.File"; Flags: uninsdeletevalue; Tasks: assoc\code; Check: NoHandler('.sql')
Root: HKA; Subkey: "Software\Classes\.yml\OpenWithProgids"; ValueType: string; ValueName: "TrorPad.File"; ValueData: ""; Flags: uninsdeletevalue; Tasks: assoc\code
Root: HKA; Subkey: "Software\Classes\Applications\trorpad.exe\SupportedTypes"; ValueType: string; ValueName: ".yml"; ValueData: ""
Root: HKA; Subkey: "Software\Classes\.yml"; ValueType: string; ValueData: "TrorPad.File"; Flags: uninsdeletevalue; Tasks: assoc\code; Check: NoHandler('.yml')
Root: HKA; Subkey: "Software\Classes\.yaml\OpenWithProgids"; ValueType: string; ValueName: "TrorPad.File"; ValueData: ""; Flags: uninsdeletevalue; Tasks: assoc\code
Root: HKA; Subkey: "Software\Classes\Applications\trorpad.exe\SupportedTypes"; ValueType: string; ValueName: ".yaml"; ValueData: ""
Root: HKA; Subkey: "Software\Classes\.yaml"; ValueType: string; ValueData: "TrorPad.File"; Flags: uninsdeletevalue; Tasks: assoc\code; Check: NoHandler('.yaml')
Root: HKA; Subkey: "Software\Classes\.csv\OpenWithProgids"; ValueType: string; ValueName: "TrorPad.File"; ValueData: ""; Flags: uninsdeletevalue; Tasks: assoc\code
Root: HKA; Subkey: "Software\Classes\Applications\trorpad.exe\SupportedTypes"; ValueType: string; ValueName: ".csv"; ValueData: ""
Root: HKA; Subkey: "Software\Classes\.csv"; ValueType: string; ValueData: "TrorPad.File"; Flags: uninsdeletevalue; Tasks: assoc\code; Check: NoHandler('.csv')

[Run]
Filename: "{app}\trorpad.exe"; Description: "{cm:LaunchProgram,TrorPad}"; Flags: nowait postinstall skipifsilent
Filename: "{app}\trorpad.exe"; Flags: nowait runasoriginaluser; Check: WizardSilent


[Code]
function NoHandler(Ext: String): Boolean;
var
  Value: String;
begin
  Result := not RegQueryStringValue(HKCR, Ext, '', Value) or (Value = '') or (Value = 'TrorPad.File');
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
var
  Dir: String;
  Code: Integer;
  Settings, Profiles, Data: Boolean;
begin
  if (CurUninstallStep <> usUninstall) or UninstallSilent then
    Exit;
  Dir := ExpandConstant('{userappdata}\Tror\data');
  if not DirExists(Dir) then
    Exit;
  Settings := MsgBox(CustomMessage('AskSettings'), mbConfirmation, MB_YESNO or MB_DEFBUTTON2) = IDYES;
  Profiles := MsgBox(CustomMessage('AskProfiles'), mbConfirmation, MB_YESNO or MB_DEFBUTTON2) = IDYES;
  Data := MsgBox(CustomMessage('AskData'), mbConfirmation, MB_YESNO or MB_DEFBUTTON2) = IDYES;
  if Profiles then
  begin
    Exec(ExpandConstant('{app}\trorpad.exe'), '--forget-passwords', '', SW_HIDE, ewWaitUntilTerminated, Code);
    DeleteFile(Dir + '\connections.json');
  end;
  if Settings then
    DeleteFile(Dir + '\app.ron');
  if Data then
  begin
    DeleteFile(Dir + '\db_history.json');
    DelTree(Dir + '\recover', True, True, True);
    DelTree(Dir + '\plugins', True, True, True);
    DelTree(Dir + '\backups', True, True, True);
  end;
  if Settings and Profiles and Data then
    DelTree(ExpandConstant('{userappdata}\Tror'), True, True, True);
end;
