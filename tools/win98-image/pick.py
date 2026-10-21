"""From treediff.txt, the files of eXo's C: that go into ours (stage/)."""
import os, re, shutil

drop = re.compile(r"""(?ix)
  ^[^/]+$                                   # anything in C:\ itself
 |^(my\ documents|recycled|exo/wallpaper|exo/archived)/
 |^exo/(qres\.htm|setup\.vbs|selectgamelist\.vbs)$
 |^program\ files/(uninstall\ information|online\ services|netmeeting|internet\ explorer|outlook\ express)/
 |^windows/(temp|recent|history|cookies|temporary\ internet\ files|applog|sysbckup|web|media|npcommon
           |downloaded\ program\ files|start\ menu|application\ data|favorites|sendto|helpdesk|desktop|local\ settings)/
 |^windows/system/precopy/
 |\.(pnf|pwl|log|txt|bak|1st|prv|swp|scr|wav|bmp|htm|gif|jpg|dat|sf0|sfc|gid|bin|cb)$
 |^windows/[^/]*\.(ini|pif)$               # merged by hand
 |^windows/(ttfcache|shelliconcache|wininit\.bak|win386\.swp)$
 |^windows/inf/(setup\.inf|drvidx\.bin|drvdata\.bin)$
""")
keep = []
for line in open("treediff.txt"):
    kind, size, path = line.rstrip("\n").split(None, 2)
    if drop.search(path) and not path.upper().startswith("GLIDE"):
        continue
    keep.append(path)
shutil.rmtree("stage", ignore_errors=True)
for p in keep:
    os.makedirs(os.path.join("stage", os.path.dirname(p)), exist_ok=True)
    shutil.copy2(os.path.join("tree/exo", p), os.path.join("stage", p))
open("stage-files.txt", "w").write("\n".join(keep) + "\n")
print(len(keep), "files")
