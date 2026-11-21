#!/usr/bin/env bash
# Install New Hera-nux, the companion app of the Hercules Stream 100, for the current user, and
# start it minimized (in the system tray) when the session opens.
#
# From a release archive (this script next to hsm01-app) or from the source tree
# (packaging/install.sh: the app is built with cargo first). Nothing is written outside your
# home, except the udev rule (with sudo) that gives you access to the device.
#
#   ./install.sh                  install, or update what is installed; start at login, udev
#                                 rule, start it now (restarted if it was running)
#   ./install.sh --keep-modules   on an update, leave your copies of the shipped modules alone
#   ./install.sh --no-autostart   do not start it when the session opens
#   ./install.sh --no-udev        leave the udev rule alone (already there, or done by hand)
#   ./install.sh --no-start       do not start it now
#   ./install.sh --uninstall      remove it (your settings and your modules stay)
#
# The app needs the custom firmware (240.xx) on the device: see « How to install » in README.md.
set -euo pipefail

autostart_on=1 udev_on=1 start_on=1 uninstall=0 modules_on=1
for arg in "$@"; do
    case "$arg" in
        --keep-modules) modules_on=0 ;;
        --no-autostart) autostart_on=0 ;;
        --no-udev) udev_on=0 ;;
        --no-start) start_on=0 ;;
        --uninstall) uninstall=1 ;;
        -h | --help) sed -n '2,17p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
        *) echo "unknown option: $arg (see --help)" >&2; exit 2 ;;
    esac
done

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
data="${XDG_DATA_HOME:-$HOME/.local/share}"
config="${XDG_CONFIG_HOME:-$HOME/.config}"
dest="$data/new-hera-nux"                  # the program, its shipped modules, its Python library
user_modules="$data/hsm01/modules"         # the modules the app runs (yours and the shipped ones)
bin="$HOME/.local/bin"
menu_entry="$data/applications/new-hera-nux.desktop"
autostart_entry="$config/autostart/new-hera-nux.desktop"
icon="$data/icons/hicolor/scalable/apps/new-hera-nux.svg"
rule_name="70-hercules-stream100.rules"
rule_dest="/etc/udev/rules.d/$rule_name"

say() { printf '\033[1;36m==>\033[0m %s\n' "$*"; }
warn() { printf '\033[1;33m!!\033[0m %s\n' "$*" >&2; }
die() { printf '\033[1;31mxx\033[0m %s\n' "$*" >&2; exit 1; }

# the running app quits cleanly on SIGTERM (the device goes to standby)
stop_app() {
    if pgrep -u "$USER" -x hsm01-app >/dev/null; then
        say "Stopping the running app"
        pkill -u "$USER" -x hsm01-app || true
        for _ in $(seq 1 50); do
            pgrep -u "$USER" -x hsm01-app >/dev/null || return 0
            sleep 0.1
        done
        warn "the app did not stop within 5 s"
    fi
}

if [[ $uninstall == 1 ]]; then
    stop_app
    say "Removing New Hera-nux"
    rm -rf "$dest"
    rm -f "$bin/hsm01-app" "$bin/hsm01ctl" "$menu_entry" "$autostart_entry" "$icon"
    command -v update-desktop-database >/dev/null && update-desktop-database "$data/applications" 2>/dev/null || true
    say "Done. Kept: your settings ($config/hsm01/) and your modules ($data/hsm01/)."
    [[ -f $rule_dest ]] && echo "    The udev rule stays; to remove it: sudo rm $rule_dest && sudo udevadm control --reload"
    exit 0
fi

# --- what to install: a release archive, or the source tree (built here) -------------------
if [[ -x "$here/hsm01-app" ]]; then
    version="$(cat "$here/VERSION" 2>/dev/null || echo unknown)"
    src_bin="$here"
    src_modules="$here/modules"
    src_python="$here/clients/python"
    src_rule="$here/$rule_name"
    src_icon="$here/new-hera-nux.svg"
elif [[ -f "$here/../app/Cargo.toml" ]]; then
    root="$(cd "$here/.." && pwd)"
    command -v cargo >/dev/null || die "cargo not found: install Rust (https://rustup.rs), or use a release archive"
    say "Building the app (cargo build --release)"
    (cd "$root/app" && cargo build --release --locked)
    version="$(sed -n 's/^version = "\(.*\)"/\1/p' "$root/app/Cargo.toml" | head -n 1)"
    described="$(git -C "$root" describe --tags --always --dirty 2>/dev/null || true)"
    [[ -n $described ]] && version="$version ($described)"
    src_bin="${CARGO_TARGET_DIR:-$root/app/target}/release"
    src_modules="$root/app/modules"
    src_python="$root/app/clients/python"
    src_rule="$root/packaging/udev/$rule_name"
    src_icon="$root/public/tray.svg"
else
    die "run this script from a release archive (next to hsm01-app) or from packaging/ of the source tree"
fi
[[ -x "$src_bin/hsm01-app" && -d "$src_modules" && -d "$src_python/hsm01" ]] \
    || die "incomplete files in $src_bin (hsm01-app, modules/, clients/python/)"

# --- what the app needs at run time ---------------------------------------------------------
missing=()
ldconfig="$(command -v ldconfig || echo /sbin/ldconfig)"   # not in a user's PATH on Debian
if [[ -x $ldconfig ]]; then
    libs="$("$ldconfig" -p 2>/dev/null || true)"
    grep -q 'libusb-1.0.so' <<<"$libs" || missing+=("libusb (libusb-1.0-0 / libusb)")
    grep -q 'libpulse.so.0' <<<"$libs" || missing+=("libpulse (libpulse0 / libpulse)")
fi
if ((${#missing[@]})); then
    warn "missing libraries: ${missing[*]}"
    warn "  Debian / Ubuntu: sudo apt install libusb-1.0-0 libpulse0   Arch / Manjaro: sudo pacman -S libusb libpulse"
fi
if command -v python3 >/dev/null; then
    read -r major minor < <(python3 --version 2>&1 | sed -E 's/^Python ([0-9]+)\.([0-9]+).*/\1 \2/')
    if ((major < 3 || (major == 3 && minor < 11))); then
        warn "Python $major.$minor: the modules need Python 3.11 or later"
    fi
else
    warn "python3 not found: the modules (Discord, Spotify, OBS…) need Python 3.11 or later"
fi

# --- the program, next to its modules and Python library (the app finds them there) -------
previous=""
if [[ -x "$dest/hsm01-app" ]]; then
    previous="$(cat "$dest/VERSION" 2>/dev/null || echo "an earlier version")"
    say "Updating New Hera-nux: $previous → $version ($dest)"
else
    say "Installing New Hera-nux $version to $dest"
fi
stop_app_after=0
pgrep -u "$USER" -x hsm01-app >/dev/null && stop_app_after=1
rm -rf "$dest.new"
mkdir -p "$dest.new/clients" "$bin"
cp "$src_bin/hsm01-app" "$dest.new/"
[[ -x "$src_bin/hsm01ctl" ]] && cp "$src_bin/hsm01ctl" "$dest.new/"
cp -r "$src_modules" "$dest.new/modules"
cp -r "$src_python" "$dest.new/clients/python"
find "$dest.new" -name __pycache__ -type d -prune -exec rm -rf {} +
[[ -f "$src_rule" ]] && cp "$src_rule" "$dest.new/"
echo "$version" > "$dest.new/VERSION"
rm -rf "$dest.old"
[[ -d $dest ]] && mv "$dest" "$dest.old"
mv "$dest.new" "$dest"
rm -rf "$dest.old"
ln -sf "$dest/hsm01-app" "$bin/hsm01-app"
[[ -x "$dest/hsm01ctl" ]] && ln -sf "$dest/hsm01ctl" "$bin/hsm01ctl"

# --- your copies of the shipped modules: their new code, your settings and secrets kept ----
# (the app copies the shipped modules you do not have yet by itself, at its start)
settings_file() {  # the settings file a module.toml names in [settings]
    awk '/^\[settings\]/ { s = 1; next } /^\[/ { s = 0 } s && /^file *=/ {
        sub(/^file *= *"/, ""); sub(/".*$/, ""); print; exit }' "$1"
}
if [[ -n $previous && $modules_on == 1 && -d $user_modules ]]; then
    updated=()
    for shipped in "$dest"/modules/*/; do
        id="$(basename "$shipped")"
        mine="$user_modules/$id"
        [[ -f "$mine/module.toml" || -f "$mine/tool.toml" ]] || continue
        keep="$(settings_file "$shipped/module.toml")"
        while IFS= read -r -d '' f; do
            rel="${f#./}"
            [[ $rel == .env ]] && continue
            [[ -n $keep && $rel == "$keep" && -e "$mine/$rel" ]] && continue
            mkdir -p "$(dirname "$mine/$rel")"
            cp "$shipped/$rel" "$mine/$rel"
        done < <(cd "$shipped" && find . -type f ! -path '*/__pycache__/*' -print0)
        updated+=("$id")
    done
    ((${#updated[@]})) && say "Shipped modules updated (your settings and secrets kept): ${updated[*]}"
fi

# --- menu entry, icon, start at login ---------------------------------------------------------
if [[ -f $src_icon ]]; then
    mkdir -p "$(dirname "$icon")"
    cp "$src_icon" "$icon"
fi
entry() {  # $1: extra arguments, $2: extra lines
    cat <<EOF
[Desktop Entry]
Type=Application
Name=New Hera-nux
GenericName=Hercules Stream 100
Comment=Companion app of the Hercules Stream 100
Comment[fr]=Application compagnon du Hercules Stream 100
Exec=$dest/hsm01-app$1
Icon=new-hera-nux
Terminal=false
Categories=AudioVideo;Audio;Mixer;
Keywords=hercules;stream;deck;mixer;knobs;
StartupNotify=false
$2
EOF
}
mkdir -p "$(dirname "$menu_entry")"
entry "" "" > "$menu_entry"
command -v update-desktop-database >/dev/null && update-desktop-database "$data/applications" 2>/dev/null || true
if [[ $autostart_on == 1 ]]; then
    say "Starting it minimized when the session opens ($autostart_entry)"
    mkdir -p "$(dirname "$autostart_entry")"
    # after the panel (its system tray), a few seconds in
    entry " --minimized" "X-GNOME-Autostart-enabled=true
X-GNOME-Autostart-Delay=5
X-KDE-autostart-after=panel
X-KDE-autostart-phase=2" > "$autostart_entry"
else
    rm -f "$autostart_entry"
fi

# --- access to the device without root --------------------------------------------------------
if [[ $udev_on == 1 && -f "$dest/$rule_name" ]]; then
    if cmp -s "$dest/$rule_name" "$rule_dest" 2>/dev/null; then
        say "udev rule already in place"
    else
        say "Installing the udev rule (sudo): access to the device and its DFU mode without root"
        sudo install -m 0644 "$dest/$rule_name" "$rule_dest"
        sudo udevadm control --reload
        sudo udevadm trigger
        echo "    Unplug and plug the Stream 100 back in."
    fi
fi

# --- start it ---------------------------------------------------------------------------------
if [[ $start_on == 1 ]]; then
    if [[ $stop_app_after == 1 ]]; then stop_app; fi
    say "Starting New Hera-nux (in the system tray)"
    setsid -f "$dest/hsm01-app" --minimized >/dev/null 2>&1 < /dev/null
elif [[ $stop_app_after == 1 ]]; then
    warn "the app runs the former version: quit it (tray icon → Quit) and start it again"
fi

if [[ -n $previous ]]; then say "Updated: $previous → $version"; else say "Installed: $version"; fi
echo "    Open it: application menu → New Hera-nux, or its icon in the system tray."
case ":$PATH:" in *":$bin:"*) ;; *) echo "    $bin is not in your PATH: add it to run hsm01-app / hsm01ctl from a terminal." ;; esac
echo "    Update later: run the install.sh of a newer release (or of the source tree) again."
echo "    The device needs the custom firmware (240.xx): README.md, « How to install », steps 3 and 4."
