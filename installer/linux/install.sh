#!/usr/bin/env bash
# Optional installation of a portable package for this user. System packages use
# their package manager instead. User settings and portable mods are preserved.
set -euo pipefail
here="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
data="${XDG_DATA_HOME:-$HOME/.local/share}"
[[ "$data" = /* && "$data" != / ]] || { echo 'XDG_DATA_HOME must be a non-root absolute path.' >&2; exit 1; }
[[ "$data" != *$'\n'* && "$data" != *$'\r'* ]] || { echo 'A newline in the data path is not supported.' >&2; exit 1; }
mkdir -p -- "$data"
data="$(cd -- "$data" && pwd -P)"
dest="$data/ta-reborn"
apps="$data/applications"
entry="$apps/ta-reborn-launcher.desktop"
# The game alone, with the launcher's settings: for Steam's "Add a Non-Steam Game" (a Steam
# Deck's Game Mode) when the launcher is not wanted in between.
game_entry="$apps/ta-reborn-game.desktop"
marker="$dest/.ta-reborn-install"
[[ ! -L "$dest" && ! -L "$apps" && ! -L "$entry" && ! -L "$game_entry" && ! -L "$marker" ]] || { echo 'Refusing a linked install or menu destination.' >&2; exit 1; }
[[ "$(realpath -m -- "$dest")" == "$data/ta-reborn" ]] || { echo 'Unsafe install destination.' >&2; exit 1; }
[[ $# -eq 0 || ( $# -eq 1 && "$1" == --uninstall ) ]] || { echo 'Usage: ./install.sh [--uninstall]' >&2; exit 1; }
items=(ta-launcher ta-reborn assets lang docs README.txt LICENSE.txt THIRD-PARTY-NOTICES.txt build-info.json ta-reborn.png install.sh)

if [[ "${1:-}" == "--uninstall" ]]; then
    [[ -f "$marker" && "$(cat -- "$marker")" == TA_REBORN_PER_USER_INSTALL_V1 ]] || {
        echo "Refusing to remove an unmarked installation: $dest" >&2; exit 1;
    }
    for item in "${items[@]}"; do
        [[ ! -L "$dest/$item" ]] || { echo "Refusing linked installed content: $dest/$item" >&2; exit 1; }
    done
    # Every target is a fixed child of the verified destination. Keep mods and all
    # other user-created files; never recursively remove the installation itself.
    for item in "${items[@]}"; do rm -rf -- "$dest/$item"; done
    rm -f -- "$marker"
    for menu in "$entry" "$game_entry"; do
        if [[ -f "$menu" ]] && grep -qx 'X-TA-Reborn-Managed=true' "$menu"; then rm -f -- "$menu"; fi
    done
    echo "TA Reborn removed. Settings, saves and mods were kept. Remaining files: $dest"
    exit 0
fi

[[ "$dest" != "$here" ]] || { echo 'This copy is already installed here.'; exit 0; }
mkdir -p -- "$dest" "$apps"
for item in "${items[@]}"; do
    [[ -e "$here/$item" && ! -L "$here/$item" && ! -L "$dest/$item" ]] || { echo "Missing or linked package item: $item" >&2; exit 1; }
    if [[ -d "$here/$item" ]] && [[ -n "$(find "$here/$item" -type l -print -quit)" ]]; then
        echo "Refusing linked package content: $here/$item" >&2; exit 1
    fi
    if [[ -d "$dest/$item" ]] && [[ -n "$(find "$dest/$item" -type l -print -quit)" ]]; then
        echo "Refusing linked installed content: $dest/$item" >&2; exit 1
    fi
done
for item in "${items[@]}"; do cp -R -- "$here/$item" "$dest/"; done
printf '%s\n' TA_REBORN_PER_USER_INSTALL_V1 > "$marker"
chmod +x -- "$dest/ta-launcher" "$dest/ta-reborn" "$dest/install.sh"
# A path for a desktop entry's Exec, quoted and escaped as the specification asks.
exec_path() {
    local path="$1"
    path="${path//\\/\\\\}"
    path="${path//\"/\\\"}"
    path="${path//\$/\\\$}"
    path="${path//\`/\\\`}"
    path="${path//%/%%}"
    printf '%s' "$path"
}
launcher="$(exec_path "$dest/ta-launcher")"
game="$(exec_path "$dest/ta-reborn")"
cat > "$entry" <<EOF
[Desktop Entry]
Type=Application
Name=TA Reborn
Comment=A new engine for Total Annihilation (needs your own copy of the game)
Exec="$launcher"
Icon=$dest/ta-reborn.png
Terminal=false
Categories=Game;StrategyGame;
X-TA-Reborn-Managed=true
EOF
cat > "$game_entry" <<EOF
[Desktop Entry]
Type=Application
Name=TA Reborn (game only)
Comment=Starts the game straight away, with the settings made in the launcher
Exec="$game"
Icon=$dest/ta-reborn.png
Terminal=false
Categories=Game;StrategyGame;
X-TA-Reborn-Managed=true
EOF
echo "Installed to $dest. Start TA Reborn from the applications menu."
echo 'Select your existing Total Annihilation data folder in the launcher.'
