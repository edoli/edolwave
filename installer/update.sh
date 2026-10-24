#!/bin/sh
# A separate process owns the downloaded package after the UI commits and exits.
set -eu
ROOT=$EDOLWAVE_UPDATE_DIR
EXE=$EDOLWAVE_UPDATE_EXE
PACKAGE=$EDOLWAVE_UPDATE_PACKAGE
PARENT=$EDOLWAVE_UPDATE_PARENT
RESULT=$EDOLWAVE_CONFIG_DIR/update-result.txt
committed=0
mount_dir=
stage=
backup=
library_stage=
library_backup=
library_target=
case "${ROOT##*/}" in edolwave-update-*) ;; *) exit 1 ;; esac
[ "${PACKAGE%/*}" = "$ROOT" ] || exit 1

verify() {
    if command -v sha256sum >/dev/null 2>&1; then
        actual=$(sha256sum "$PACKAGE" | cut -d ' ' -f 1)
    else
        actual=$(shasum -a 256 "$PACKAGE" | cut -d ' ' -f 1)
    fi
    [ "$actual" = "$EDOLWAVE_UPDATE_HASH" ]
}

finish() {
    status=$?
    trap - EXIT
    if [ "$status" -ne 0 ]; then
        printf 'Update failed. %s\n' "$(tail -c 2048 "$ROOT/install.log" 2>/dev/null || true)" > "$RESULT"
        if [ -n "$backup" ] && [ -e "$backup" ]; then
            mv "$backup" "$EXE" || true
        fi
        if [ -n "$library_backup" ] && [ -d "$library_backup" ]; then
            # The target is the app-owned usr/lib directory, never a system lib directory.
            if [ -d "$library_target" ]; then mv "$library_target" "$ROOT/failed-libraries"; fi
            mv "$library_backup" "$library_target" || true
        fi
    else
        rm -f "$RESULT"
    fi
    if [ -n "$mount_dir" ]; then hdiutil detach "$mount_dir" -quiet || true; fi
    if [ "$committed" -eq 1 ]; then
        nohup sh "$ROOT/restart.sh" >/dev/null 2>&1 &
        # Leave restart.sh until its child has opened it.
        sleep 1
    fi
    if [ -n "$stage" ] && [ -f "$stage" ]; then rm -f "$stage"; fi
    if [ -n "$backup" ] && [ -f "$backup" ]; then rm -f "$backup"; fi
    # All extraction directories are freshly created children of this update's root.
    for owned in "$ROOT/payload" "$ROOT/failed-libraries"; do
        case "$owned" in "$ROOT"/*) rm -rf "$owned" ;; esac
    done
    for owned in "$library_stage" "$library_backup"; do
        case "$owned" in "${EXE%/usr/bin/edolwave}/usr/".edolwave-*) rm -rf "$owned" ;; esac
    done
    rm -f "$ROOT/context.json" "$ROOT/update.sh" "$ROOT/install-mac.sh" "$ROOT/restart.sh" "$ROOT/ready" "$ROOT/go" "$ROOT/go.pending" "$PACKAGE"
    if [ -n "$mount_dir" ]; then rmdir "$mount_dir" 2>/dev/null || true; fi
    if [ "$status" -eq 0 ]; then rm -f "$ROOT/install.log"; fi
    rmdir "$ROOT" 2>/dev/null || true
}
trap finish EXIT
exec 2>"$ROOT/install.log"
verify
case "${PACKAGE##*/}" in
    edolwave-linux.zip) command -v unzip >/dev/null 2>&1 ;;
esac
printf ready > "$ROOT/ready"
while [ ! -f "$ROOT/go" ]; do
    if ! kill -0 "$PARENT" 2>/dev/null; then
        [ -f "$ROOT/go" ] || exit 0
    fi
    sleep 0.05
done
committed=1
while kill -0 "$PARENT" 2>/dev/null; do sleep 0.1; done
verify
case "${PACKAGE##*/}" in
    edolwave-linux.zip|edolwave-mac.zip)
        stage=$(mktemp "${EXE}.update.XXXXXX")
        if [ "${PACKAGE##*/}" = 'edolwave-mac.zip' ]; then
            unzip -p "$PACKAGE" EdolWave.app/Contents/MacOS/edolwave > "$stage"
        else
            unzip -p "$PACKAGE" edolwave > "$stage"
        fi
        [ -s "$stage" ]
        chmod 755 "$stage"
        backup=$(mktemp "${EXE}.old.XXXXXX")
        cp -p "$EXE" "$backup"
        mv -f "$stage" "$EXE"
        stage=
        ;;
    edolwave-installer-appimage.AppImage)
        stage=$(mktemp "${EXE}.update.XXXXXX")
        cp "$PACKAGE" "$stage"
        chmod 755 "$stage"
        mv -f "$stage" "$EXE"
        stage=
        ;;
    edolwave-installer-deb.deb)
        [ "$(dpkg-deb -f "$PACKAGE" Version)" = "$EDOLWAVE_UPDATE_VERSION" ]
        pkexec /usr/bin/dpkg -i "$PACKAGE"
        ;;
    edolwave-installer-dmg.dmg)
        case "$EXE" in */Contents/MacOS/edolwave) ;; *) echo 'Not an app bundle' >&2; exit 1 ;; esac
        bundle=${EXE%/Contents/MacOS/edolwave}
        mount_dir=$ROOT/mount
        mkdir "$mount_dir"
        hdiutil attach -nobrowse -quiet -mountpoint "$mount_dir" "$PACKAGE"
        codesign --verify --deep --strict "$mount_dir/EdolWave.app"
        [ "$(defaults read "$mount_dir/EdolWave.app/Contents/Info" CFBundleShortVersionString)" = "$EDOLWAVE_UPDATE_VERSION" ]
        # ditto handles app-bundle metadata. Authorization, when needed, is
        # supplied by macOS; no password is read or stored by the application.
        cat > "$ROOT/install-mac.sh" <<'MACINSTALL'
#!/bin/sh
set -eu
source_app=$1
target_app=$2
parent=${target_app%/*}
work=$(mktemp -d "$parent/.edolwave-update.XXXXXX")
cleanup() {
    if [ -d "$work/previous.app" ] && [ ! -d "$target_app" ]; then mv "$work/previous.app" "$target_app"; fi
    case "$work" in "$parent/".edolwave-update.*) rm -rf "$work" ;; esac
}
trap cleanup EXIT
ditto "$source_app" "$work/new.app"
codesign --verify --deep --strict "$work/new.app"
mv "$target_app" "$work/previous.app"
mv "$work/new.app" "$target_app"
MACINSTALL
        osascript - "$ROOT/install-mac.sh" "$mount_dir/EdolWave.app" "$bundle" <<'APPLESCRIPT'
on run argv
    set installScript to item 1 of argv
    set sourceApp to item 2 of argv
    set targetApp to item 3 of argv
    set operation to "/bin/sh " & quoted form of installScript & " " & quoted form of sourceApp & " " & quoted form of targetApp
    try
        do shell script operation
    on error
        do shell script operation with administrator privileges
    end try
end run
APPLESCRIPT
        ;;
    *) echo 'Unsupported update package' >&2; exit 1 ;;
esac
