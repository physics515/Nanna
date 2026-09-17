#!/bin/sh
# Cargo `runner` for Linux: exec a cargo-built binary with the AppImage
# bundle's own directories removed from LD_LIBRARY_PATH.
#
# When Nanna runs from an AppImage, AppRun exports LD_LIBRARY_PATH pointing at
# the bundle's libraries, and every child — including a `cargo test` the agent
# runs through `exec` — inherits it. A freshly built test binary then loads the
# bundle's older liblzma/libssl instead of the system's and dies at load with
# "version `XZ_5.4' not found". The scripted `exec` tool scrubs this for the
# children it spawns (nanna-scripting/src/bridge.rs, `scrub_appimage_library_path`);
# this runner applies the same rule at cargo's own execution hook, which is the
# one place that covers a binary cargo launches itself.
#
# Same rule as the bridge: only entries under $APPDIR are dropped, everything
# the user set stays. Outside an AppImage ($APPDIR unset) this is a plain exec.
if [ -n "${APPDIR:-}" ] && [ -n "${LD_LIBRARY_PATH:-}" ]; then
    kept=""
    old_ifs=$IFS
    IFS=:
    for entry in $LD_LIBRARY_PATH; do
        case "$entry" in
            "$APPDIR"|"$APPDIR"/*) ;;
            "") ;;
            *) kept="${kept:+$kept:}$entry" ;;
        esac
    done
    IFS=$old_ifs
    if [ -n "$kept" ]; then
        LD_LIBRARY_PATH=$kept
        export LD_LIBRARY_PATH
    else
        unset LD_LIBRARY_PATH
    fi
fi
exec "$@"
