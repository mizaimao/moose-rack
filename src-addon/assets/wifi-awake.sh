# moose-patch: stop the wireless dropping while idle.
case "$1" in
  start)
    for dev in /sys/class/net/wlan*; do
      [ -e "$dev" ] || continue
      iw dev "$(basename "$dev")" set power_save off 2>/dev/null
    done
    ;;
esac
