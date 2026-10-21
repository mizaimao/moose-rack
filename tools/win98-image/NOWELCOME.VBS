Set sh = CreateObject("WScript.Shell")
' Windows 98 starts its Welcome screen, music and all, from this entry.
' REGEDIT 4 cannot delete a value, so it is done here.
sh.RegDelete "HKLM\Software\Microsoft\Windows\CurrentVersion\Run\Welcome"
