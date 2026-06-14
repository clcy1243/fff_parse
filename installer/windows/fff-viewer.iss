; FFF Viewer Windows 安装包定义（Inno Setup 6）
; 版本号由 build-windows.ps1 经 /DMyAppVersion= 注入；未注入时回退占位值。
#define MyAppName "FFF Viewer"
#ifndef MyAppVersion
  #define MyAppVersion "0.0.0"
#endif
#define MyAppExeName "fff_viewer.exe"
#define MyAppPublisher "clcy.me"

[Setup]
; AppId 为本产品固定 GUID，勿随版本更改（升级识别依赖它）
AppId={{B7E4B0A2-5C3D-4F1A-9E2B-7A6C8D9E0F11}
AppName={#MyAppName}
AppVersion={#MyAppVersion}
AppPublisher={#MyAppPublisher}
DefaultDirName={autopf}\FFF Viewer
DefaultGroupName=FFF Viewer
DisableProgramGroupPage=yes
UninstallDisplayIcon={app}\{#MyAppExeName}
OutputDir=..\..\dist
OutputBaseFilename=FFF Viewer-{#MyAppVersion}-setup
SetupIconFile=..\..\icons\icon.ico
Compression=lzma2
SolidCompression=yes
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
WizardStyle=modern

[Languages]
; ChineseSimplified.isl 随仓库提供（与本 .iss 同目录），不依赖构建机的 Inno
; 是否自带该语言文件，避免缺失时 ISCC 编译中断。
Name: "chinesesimp"; MessagesFile: "ChineseSimplified.isl"
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "创建桌面快捷方式"; GroupDescription: "附加任务:"; Flags: unchecked

[Files]
Source: "..\..\target\release\fff_viewer.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\..\profiles\*"; DestDir: "{app}\profiles"; Flags: ignoreversion recursesubdirs createallsubdirs
Source: "..\..\settings\*"; DestDir: "{app}\settings"; Flags: ignoreversion recursesubdirs createallsubdirs

[Icons]
Name: "{group}\FFF Viewer"; Filename: "{app}\{#MyAppExeName}"
Name: "{group}\卸载 FFF Viewer"; Filename: "{uninstallexe}"
Name: "{autodesktop}\FFF Viewer"; Filename: "{app}\{#MyAppExeName}"; Tasks: desktopicon

[Run]
Filename: "{app}\{#MyAppExeName}"; Description: "启动 FFF Viewer"; Flags: nowait postinstall skipifsilent
