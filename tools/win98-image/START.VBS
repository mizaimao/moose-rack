' Moose Rack, Windows 98: runs at every logon and starts the game on D:.
' D: is laid out as eXoWin9x packs are: files for C:\Windows under
' D:\Windows, the game's registry in D:\reg.reg, and its shortcut in
' D:\Windows\Desktop. With no D:, it does nothing.
Option Explicit
Dim fso, sh, file, games, names, n, pick
Set fso = CreateObject("Scripting.FileSystemObject")
Set sh = CreateObject("WScript.Shell")

' File by file rather than CopyFolder, which stops at the first file it cannot
' write and takes the registry import and the launch below with it. That
' happened on a second logon in the same tab: the first had copied the game's
' read-only files, and a copy will not overwrite a read-only file. So the
' read-only bit on an existing copy is cleared first, and a file that still
' fails is skipped on its own. SYSBCKUP is left out: eXo's old registry
' backups, which no game needs, under the names ScanRegW gives its own.
Sub CopyTree(src, dst)
    Dim f, d, t, a
    On Error Resume Next
    If Not fso.FolderExists(dst) Then fso.CreateFolder dst
    For Each f In fso.GetFolder(src).Files
        t = dst & "\" & f.Name
        If fso.FileExists(t) Then
            a = fso.GetFile(t).Attributes
            ' 39 is the bits a script may set: read-only, hidden, system, archive.
            If a And 1 Then fso.GetFile(t).Attributes = (a And 39) And Not 1
        End If
        f.Copy t, True
        Err.Clear
    Next
    For Each d In fso.GetFolder(src).SubFolders
        If LCase(d.Name) <> "sysbckup" Then CopyTree d.Path, dst & "\" & d.Name
    Next
End Sub

' The lines of one [section] of an INI file, or "" when it has none.
Function SectionText(path, name)
    Dim f, line, inside, out
    SectionText = ""
    If Not fso.FileExists(path) Then Exit Function
    Set f = fso.OpenTextFile(path, 1)
    inside = False
    Do While Not f.AtEndOfStream
        line = f.ReadLine
        If Left(Trim(line), 1) = "[" Then
            inside = (LCase(Trim(line)) = "[" & LCase(name) & "]")
        ElseIf inside And Trim(line) <> "" Then
            out = out & line & vbCrLf
        End If
    Loop
    f.Close
    SectionText = out
End Function

' Make `body` the whole of [name] in an INI file.
Sub SetSection(path, name, body)
    Dim f, line, inside, out, done, a
    On Error Resume Next
    If Not fso.FileExists(path) Then Exit Sub
    Set f = fso.OpenTextFile(path, 1)
    inside = False
    done = False
    Do While Not f.AtEndOfStream
        line = f.ReadLine
        If Left(Trim(line), 1) = "[" Then
            inside = (LCase(Trim(line)) = "[" & LCase(name) & "]")
            out = out & line & vbCrLf
            If inside Then
                out = out & body
                done = True
            End If
        ElseIf Not inside Then
            out = out & line & vbCrLf
        End If
    Loop
    f.Close
    If Not done Then out = out & "[" & name & "]" & vbCrLf & body
    a = fso.GetFile(path).Attributes
    If a And 1 Then fso.GetFile(path).Attributes = (a And 39) And Not 1
    Set f = fso.CreateTextFile(path, True)
    f.Write out
    f.Close
End Sub

' A game's SYSTEM.INI is eXo's own with the game's lines added, and its
' [Password Lists] names eXo's user, not ours. Copied over as it is, Windows
' finds no password list for our user and asks for a logon at the next start.
' So ours is kept and put back after the copy.
Dim passwords
passwords = SectionText("C:\Windows\SYSTEM.INI", "Password Lists")
If fso.FolderExists("D:\Windows") Then CopyTree "D:\Windows", "C:\Windows"
If passwords <> "" Then SetSection "C:\Windows\SYSTEM.INI", "Password Lists", passwords
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
