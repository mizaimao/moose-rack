# Moose Rack: clear the boot splash out of the framebuffer.
# S03system-splash paints the KNULLI logo into /dev/fb0 and leaves it there.
# It is invisible while ES or an emulator owns a DRM plane, and flashes up
# every time one is torn down -- so on every game launch and every exit.
case "$1" in
  start)
    if [ -e /dev/fb0 ] && [ -r /sys/class/graphics/fb0/virtual_size ]; then
      W=$(cut -d, -f1 /sys/class/graphics/fb0/virtual_size)
      H=$(cut -d, -f2 /sys/class/graphics/fb0/virtual_size)
      dd if=/dev/zero of=/dev/fb0 bs=4096 count=$(( W * H * 4 / 4096 )) 2>/dev/null
    fi
    ;;
esac
