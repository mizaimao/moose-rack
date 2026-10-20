"""Pick what moves from eXo's Windows 98 into ours: files and registry.

Writes trans-files.txt (paths inside the image) and TRANS.REG (REGEDIT4).
Everything eXo added that games depend on; nothing that is only its look,
its sounds, its user, or traces of its own hardware.
"""
import re
import w9xreg

known = set(open("cab-names.txt").read().split())

# ---- files ---------------------------------------------------------------
files, cur = [], {}
for line in open("exo-files.txt", errors="replace"):
    line = line.rstrip("\n")
    if line.startswith("Path = "):
        if cur:
            files.append(cur)
        cur = {"path": line[7:]}
    elif " = " in line and cur:
        k, v = line.split(" = ", 1)
        cur[k] = v
files.append(cur)
files = [f["path"] for f in files if f.get("Folder") == "-" and "Short Name" in f]

skip_dirs = re.compile(r"(?i)^(windows/(temp|sysbckup|applog|history|temporary internet files|cookies|recent|favorites|start menu|local settings|application data|web|tasks|command|media|sendto|npcommon|downloaded program files|inf/.*\.pnf$)|program files/(online services|netmeeting|uninstall information)|eXo/(wallpaper|media)/)")
skip_names = re.compile(r"(?i)(\.pwl|\.log|\.1st|\.prv|\.swp|\.scr|\.icm|\.scf|\.txt|\.bin|ttfcache|shelliconcache|win\.com|io\.sys|msdos\.sys|autoexec\.bat|config\.sys|desk\.cpl|\.htm|\.gif|\.jpg|\.wav|\.bmp|\.cab)$")
keep_anyway = re.compile(r"(?i)^windows/inf/other/")
chosen = []
for p in files:
    name = p.split("/")[-1]
    if name.lower() in known:
        continue
    if keep_anyway.search(p):
        chosen.append(p)
        continue
    if skip_dirs.search(p) or skip_names.search(name):
        continue
    if p.lower().startswith("program files/common files/") and "microsoft shared/dao" not in p.lower():
        continue
    chosen.append(p)
open("trans-files.txt", "w").write("\n".join(chosen) + "\n")

# ---- registry ------------------------------------------------------------
exo = w9xreg.read_creg("regs/exo/SYSTEM.DAT", "HKEY_LOCAL_MACHINE")
exo.update(w9xreg.read_creg("regs/exo/USER.DAT", "HKEY_USERS"))
ours = w9xreg.read_reg("regs/ours/FULL.REG")

HKLM = "HKEY_LOCAL_MACHINE\\"
include = [HKLM + p for p in [
    r"Software\3dfx Interactive", r"Software\Apple Computer, Inc.", r"Software\Intel Corporation",
    r"Software\CLASSES",
    r"Software\Microsoft\Code Store Database", r"Software\Microsoft\Jet", r"Software\Microsoft\DirectX",
    r"Software\Microsoft\DirectInput", r"Software\Microsoft\DirectDraw", r"Software\Microsoft\DirectPlay",
    r"Software\Microsoft\DirectMusic", r"Software\Microsoft\Direct3D", r"Software\Microsoft\Advanced INF Setup",
    r"Software\Microsoft\Shared Tools", r"Software\Microsoft\VBA", r"Software\Microsoft\Visual Basic",
    r"Software\Microsoft\Windows\CurrentVersion\App Paths", r"Software\Microsoft\Windows\CurrentVersion\SharedDLLs",
    r"Software\Microsoft\Windows\CurrentVersion\Uninstall",
    r"Software\Microsoft\Windows\CurrentVersion\Setup\Migration",
    r"Software\Microsoft\Windows\CurrentVersion\Setup\SetupX\INF\OEM Name",
    r"Software\Microsoft\Windows\CurrentVersion\Shell Extensions\Approved",
    r"Software\Microsoft\Windows\CurrentVersion\Controls Folder\Display\shellex\PropertySheetHandlers\3dfx",
    r"System\CurrentControlSet\Services\Class\Display\0001",
    r"System\CurrentControlSet\Services\Class\MEDIA\0003",
    r"System\CurrentControlSet\Control\MediaResources\icm",
    r"System\CurrentControlSet\Control\MediaResources\mci\MPEGVideo",
    r"System\CurrentControlSet\Control\DeviceClasses",
    r"System\CurrentControlSet\Control\SessionManager\Known16DLLs",
    r"Enum\PCI\VEN_121A&DEV_0001&SUBSYS_00000000&REV_02",
    r"Enum\PCI\VEN_5333&DEV_8811&SUBSYS_00000000&REV_00\BUS_00&DEV_01&FUNC_00",
]] + [r"HKEY_USERS\.DEFAULT\Software\Intel"]
exclude_classes = {".htm", "htmlfile", "mhtmlfile", "my documents", ".mydocs", "applications", "cplfile",
                   "dllfile", "drvfile", "scrfile", "vxdfile", "lnkfile", "x-internet-signup", ".ins", ".isp"}

def wanted(path):
    up = path.upper()
    if not any(up == i.upper() or up.startswith(i.upper() + "\\") for i in include):
        return False
    if up.startswith((HKLM + "SOFTWARE\\CLASSES\\").upper()):
        name = path.split("\\")[3].lower()
        if name in exclude_classes:
            return False
    return True

out = ["REGEDIT4", ""]
nkeys = nvals = 0
for k, (path, vals) in sorted(exo.items(), key=lambda kv: kv[0]):
    if not wanted(path):
        continue
    mine = ours.get(k, (path, {}))[1]
    lines = []
    for vk, (vname, vtype, vdata) in vals.items():
        text = w9xreg.as_text(vtype, vdata)
        if vk in mine and mine[vk][1].lower() == text.lower():
            continue
        name = "@" if vname == "" else '"' + vname.replace("\\", "\\\\").replace('"', '\\"') + '"'
        lines.append(f"{name}={text}")
    if k in ours and not lines:
        continue
    out.append(f"[{path}]")
    out.extend(lines)
    out.append("")
    nkeys += 1
    nvals += len(lines)
open("TRANS.REG", "w", encoding="latin-1").write("\r\n".join(out) + "\r\n")
print(f"files: {len(chosen)}; registry: {nkeys} keys, {nvals} values")
