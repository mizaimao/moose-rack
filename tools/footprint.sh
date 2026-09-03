#!/bin/zsh
# Total physical footprint of the app: the Rust process plus the WebKit helpers
# it spawned — and nothing else.
#
# Two ways of finding the helpers have already been wrong. Guessing `pid+8`
# missed them whenever anything else on the machine had opened a web view, and
# a run with no WebContent in it reads as 54 MB and looks like a triumph.
# Taking every WebKit process on the machine counted Safari's and read as
# 2,476 MB. So: the caller records which ones existed before the app started,
# and this counts the ones that did not.
#
# It also says when the count is not four. A silent miscount is the only way
# these numbers can lie, and they have lied twice.
BEFORE=$1
TOT=0
N=0
for p in $(pgrep -f "MacOS/moose-gui"; pgrep -f "WebKit\.(WebContent|GPU|Networking)"); do
  if [ -n "$BEFORE" ] && grep -qx "$p" $BEFORE 2>/dev/null; then continue; fi
  f=$(vmmap -summary $p 2>/dev/null | awk '/^Physical footprint:/{print $3}')
  [ -z "$f" ] && continue
  n=$(ps -o comm= -p $p 2>/dev/null | sed 's|.*/||')
  v=$(echo $f | sed 's/M//;s/K/\/1024/' | bc -l 2>/dev/null)
  TOT=$(echo "$TOT + $v" | bc -l)
  N=$((N+1))
  printf "    %-34s %8.1f MB\n" "$n" "$v"
done
[ "$N" -ne 4 ] && printf "    !! counted %d processes, expected 4 !!\n" "$N"
printf "  %-36s %8.1f MB\n" "TOTAL" "$TOT"
