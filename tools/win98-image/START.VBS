' Moose Rack, Windows 98: runs at every logon and starts the game on D:.
' D: is laid out as eXoWin9x packs are: files for C:\Windows under
' D:\Windows, the game's registry in D:\reg.reg, and its shortcut in
' D:\Windows\Desktop. With no D:, it does nothing.
Option Explicit
Dim fso, sh, file, games, names, n, pick
Set fso = CreateObject("Scripting.FileSystemObject")
Set sh = CreateObject("WScript.Shell")

If fso.FolderExists("D:\Windows") Then fso.CopyFolder "D:\Windows", "C:\Windows", True
If fso.FileExists("D:\reg.reg") Then sh.Run "regedit /s D:\reg.reg", 0, True
If Not fso.FolderExists("D:\Windows\Desktop") Then WScript.Quit

Set games = CreateObject("Scripting.Dictionary")
n = 0
For Each file In fso.GetFolder("D:\Windows\Desktop").Files
    Select Case LCase(fso.GetExtensionName(file.Name))
    Case "lnk", "vbs"
        n = n + 1
        games.Add CStr(n), file.Path
        names = names & n & ". " & fso.GetBaseName(file.Name) & vbCrLf
    End Select
Next

If n = 0 Then WScript.Quit
If n = 1 Then
    pick = "1"
Else
    pick = InputBox("Which one?" & vbCrLf & vbCrLf & names, "Start")
End If
If games.Exists(pick) Then sh.Run """" & games(pick) & """"
