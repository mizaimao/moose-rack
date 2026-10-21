"""Write the files for an unattended Windows 98 SE install in DOSBox-X.

Run in /tmp/w98build with eXo's Zulu War Play.conf beside it as exo-play.conf.
The hardware comes from that config, so Windows is installed on the PC every
eXo game disk expects. The product key is read from key1.txt (or the file
named by KEY), and goes only into files/MSBATCH.INF.
"""
import os
import re
from pathlib import Path

here = Path("/tmp/w98build")
key = (here / os.environ.get("KEY", "key1.txt")).read_text().strip()

src = (here / "exo-play.conf").read_text(encoding="latin-1").replace("\r\n", "\n")
head = src[: re.search(r"(?mi)^\[autoexec\]\s*$", src).start()]

# What the install changes from eXo's hardware, by section and key.
changes = {
    "[sdl]": {"output": None, "autolock": "false", "fullscreen": "false"},
    "[log]": {"logfile": str(here / "dosbox.log")},
    "[cpu]": {"core": "dynamic", "cycles": "max"},
    # No 3dfx card during the install: Windows 98 has no driver for it, and the
    # card and its driver go in together afterwards.
    "[voodoo]": {"voodoo_card": "false"},
    # No network card: the browser has none, and a card Windows learnt about
    # here would be "missing" on every later start.
    "[ne2000]": {"ne2000": "false"},
    "[dos]": {"hard drive data rate limit": "0", "ver": "7.1"},
}
out, section = [], ""
for line in head.split("\n"):
    s = line.strip()
    if s.startswith("[") and s.endswith("]"):
        section = s.lower()
    if "=" in s and not s.startswith("#"):
        k = s.split("=", 1)[0].strip().lower()
        if k in changes.get(section, {}):
            v = changes[section][k]
            if v is None:
                continue
            line = f"{k} = {v}"
    out.append(line.rstrip())
base = "\n".join(out).rstrip() + "\n"

img = here / "w98.img"
iso = here / "w98.iso"
files = here / "files"

(here / "make.conf").write_text(base + f"""
[autoexec]
IMGMAKE {img} -t hd -size 2047 -fat 32
IMGMOUNT C {img}
IMGMOUNT D {iso} -t cdrom
MOUNT E {files}
MD C:\\WIN98
COPY D:\\WIN98\\*.* C:\\WIN98
COPY E:\\MSBATCH.INF C:\\
MD C:\\W98POST
COPY E:\\W98POST\\*.* C:\\W98POST
EXIT
""")

# Every reboot comes back here. Once Setup has written Windows to C:, it is
# C: that boots, until Windows powers the machine off at the end.
(here / "install.conf").write_text(base + f"""
[autoexec]
IMGMOUNT C {img}
IMGMOUNT D {iso} -t cdrom -ide 2m
IF EXIST C:\\WINDOWS\\WIN.COM GOTO BOOTC
C:
CD \\WIN98
SETUP.EXE C:\\MSBATCH.INF /IS /IQ /IM /NM /NF /IE /IW
GOTO END
:BOOTC
BOOT C:
:END
""")

msbatch = f"""[BatchSetup]
Version=3.0 (32-bit)
SaveDate=10/19/26

[Version]
Signature = "$CHICAGO$"
AdvancedINF=2.0

[Setup]
Express=1
InstallDir="C:\\WINDOWS"
InstallType=1
ProductKey="{key}"
EBD=0
ShowEula=0
ChangeDir=0
OptionalComponents=1
Network=0
System=0
CCP=0
CleanBoot=0
Display=0
DevicePath=0
NoDirWarn=1
TimeZone="Eastern"
Uninstall=0
VRC=0
NoPrompt2Boot=1

[System]
Locale=L0409

[NameAndOrg]
Name="Frank"
Org="Moose Rack"
Display=0

[Network]
ComputerName="MOOSE98"
Workgroup="WORKGROUP"
Display=0

[Install]
AddReg=MooseRunOnce

[MooseRunOnce]
HKLM,"Software\\Microsoft\\Windows\\CurrentVersion\\RunOnce","MoosePost",,"C:\\W98POST\\POST.BAT"
"""
(files / "MSBATCH.INF").write_text(msbatch.replace("\n", "\r\n"))

post_reg = r"""REGEDIT4

; No startup sound.
[HKEY_CURRENT_USER\AppEvents\Schemes\Apps\.Default\SystemStart\.Current]
@=""

[HKEY_USERS\.DEFAULT\AppEvents\Schemes\Apps\.Default\SystemStart\.Current]
@=""

; No "Welcome to Windows 98" screen.
[HKEY_CURRENT_USER\Software\Microsoft\Windows\CurrentVersion\Applets\Welcome\WinStart]
"Show"=hex:00,00,00,00

[HKEY_USERS\.DEFAULT\Software\Microsoft\Windows\CurrentVersion\Applets\Welcome\WinStart]
"Show"=hex:00,00,00,00
"""
(files / "W98POST" / "POST.REG").write_text(post_reg.replace("\n", "\r\n"))

post_bat = r"""@ECHO OFF
REGEDIT /S C:\W98POST\POST.REG
ECHO done> C:\W98POST\DONE.TXT
RUNDLL32.EXE SHELL32.DLL,SHExitWindowsEx 1
"""
(files / "W98POST" / "POST.BAT").write_text(post_bat.replace("\n", "\r\n"))
print("written:", ", ".join(p.name for p in [here / "make.conf", here / "install.conf", files / "MSBATCH.INF"]))
