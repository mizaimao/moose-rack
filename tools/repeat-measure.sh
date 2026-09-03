#!/bin/zsh
# Several passes per setting, so the spread is visible instead of being
# mistaken for a result. Every pass starts from the same screen and loads no
# artwork at all, so the only thing that varies is the setting under test.
cd /Users/frank/Projects/moose-rack
S=/private/tmp/claude-501/-Users-frank-Projects-moose-rack/94af6568-3e33-4443-9b25-8971f8a269ab/scratchpad
for PASS in 1 2 3; do
for MODE in "no-covers" "no-covers,no-glass"; do
  TAG=${MODE//,/_}
  pkill -f "MacOS/moose-gui"
  while pgrep -f "MacOS/moose-gui" > /dev/null; do sleep 1; done
  sleep 6
  # Which WebKit processes belong to something else. Anything not in this list
  # afterwards is ours.
  pgrep -f "WebKit\.(WebContent|GPU|Networking)" > $S/before_${TAG}_${PASS}.pids
  MOOSE_MEASURE_FLAGS=$MODE MOOSE_MEASURE=$PWD/tools/browse.js \
    "./Moose Rack.app/Contents/MacOS/moose-gui" > $S/r_${TAG}_$PASS.log 2>&1 &
  APP=$!
  sleep 8
  for i in $(seq 1 42); do
    N=$(grep MEASURE $S/r_${TAG}_$PASS.log | tail -1 | sed 's/MEASURE //')
    echo "$MODE|$PASS|$N|$($S/footprint.sh $S/before_${TAG}_${PASS}.pids | tr '\n' ' ' | tr -s ' ')"
    sleep 1
  done
  kill $APP 2>/dev/null
done
done
