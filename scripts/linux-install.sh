#!/bin/bash
set -euo pipefail

# Default to building from the local checkout. Pass --clone if you want to build
# from a fresh GitHub clone instead (useful for CI).
USE_LOCAL=true
if [[ "${1:-}" == "--clone" ]]; then
    USE_LOCAL=false
    echo "Using fresh GitHub clone instead of local checkout"
fi

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[94m'
CYAN='\033[38;2;61;181;255m'
NC='\033[0m'

BUILD_DIR=""
DOWNLOAD_DIR=""
CLEANUP_DONE=false
FLATPAK_BUNDLE=""
PREEXIST_FLATPAK=false

cleanup() {
    local exit_code=${1:-$?}

    if [[ "$CLEANUP_DONE" == true ]]; then
        return
    fi
    CLEANUP_DONE=true

    if [[ "$exit_code" -ne 0 ]]; then
        if [[ -n "$FLATPAK_BUNDLE" && "$PREEXIST_FLATPAK" == false && -e "$FLATPAK_BUNDLE" ]]; then
            rm -f "$FLATPAK_BUNDLE" || true
        fi
    fi

    if [[ "$USE_LOCAL" == false && -n "$BUILD_DIR" && -d "$BUILD_DIR" ]]; then
        if [[ "$exit_code" -eq 0 ]]; then
            echo -e "${YELLOW}4. Cleaning up temporary directory...${NC}"
        else
            echo -e "${YELLOW}Cleaning up temporary directory...${NC}"
        fi
        rm -rf "$BUILD_DIR"
    fi

    if [[ -n "$DOWNLOAD_DIR" && -d "$DOWNLOAD_DIR" ]]; then
        rm -rf "$DOWNLOAD_DIR"
    fi
}

on_interrupt() {
    echo -e "\n${YELLOW}Cancellation received (CTRL+C). Cleaning up...${NC}"
    cleanup 130
    exit 130
}

trap on_interrupt INT TERM
trap 'cleanup $?' EXIT

echo -e "${CYAN}"
cat << "EOF"
    ____  __  _____  ___            ____           __        ____
   / __ )/  |/  /  |/  /           /  _/___  _____/ /_____ _/ / /
  / __  / /|_/ / /|_/ /  ______    / // __ \/ ___/ __/ __ `/ / /
 / /_/ / /  / / /  / /  /_____/  _/ // / / (__  ) /_/ /_/ / / /
/_____/_/  /_/_/  /_/           /___/_/ /_/____/\__/\__,_/_/_/

EOF
echo -e "${NC}"

echo -e "${GREEN}Balatro Mod Manager Linux Builder & Installer${NC}"
echo "---------------------------------------------"
echo "Started at $(date)"

# Ensure script runs under Linux
if [[ "$OSTYPE" != "linux-gnu"* && "$OSTYPE" != "linux"* ]]; then
    echo -e "${RED}This script is for Linux only.${NC}"
    exit 1
fi

# Require git
if ! command -v git &>/dev/null; then
    echo -e "${RED}git not found. Please install git.${NC}"
    exit 1
fi

# Require curl
if ! command -v curl &>/dev/null; then
    echo -e "${RED}curl not found. Please install curl and try again.${NC}"
    exit 1
fi

# Require flatpak
if ! command -v flatpak &>/dev/null; then
    echo -e "${RED}Flatpak not found. Please install flatpak and try again.${NC}"
    exit 1
fi

echo -e "${GREEN}Flatpak ✓${NC}"

ensure_flathub_remote() {
    if ! flatpak remote-list --user --columns=name --no-header 2>/dev/null | grep -qx "flathub"; then
        echo -e "${YELLOW}Adding Flathub remote...${NC}"
        flatpak remote-add --user --if-not-exists flathub https://flathub.org/repo/flathub.flatpakrepo
    else
        flatpak remote-modify --user --enable flathub >/dev/null 2>&1 || true
    fi

    # Update flathub remote metadata to ensure we have the latest runtime info
    echo -e "${YELLOW}Updating Flathub remote metadata...${NC}"
    if ! flatpak update --user --appstream flathub 2>/dev/null; then
        # If appstream update fails, try a full remote update
        echo -e "${YELLOW}Updating Flathub remote...${NC}"
        flatpak remote-modify --user --url=https://flathub.org/repo/flathub.flatpakrepo flathub 2>/dev/null || true
    fi
}

############################################
# SELECT SOURCE (LOCAL OR GITHUB CLONE)
############################################
if [[ "$USE_LOCAL" == true ]]; then
    echo -e "${YELLOW}Using local repository source${NC}"
    SCRIPT_DIR=$(dirname "$(realpath "$0")")
    REPO_DIR="$SCRIPT_DIR/.."
    cd "$REPO_DIR"
    echo -e "${GREEN}Current local repo path:${NC} $REPO_DIR"
    echo -e "${GREEN}Branch selected:${NC} $(git branch --show-current || echo 'unknown')"
else
    BUILD_DIR=$(mktemp -d)
    echo -e "${YELLOW}Using temporary directory: $BUILD_DIR${NC}"
    echo -e "${YELLOW}1. Cloning repository...${NC}"
    git clone https://github.com/rursache/balatro-mod-manager.git "$BUILD_DIR/bmm"
    REPO_DIR="$BUILD_DIR/bmm"
    cd "$REPO_DIR"
fi

############################################
# BUILD + INSTALL FLATPAK
############################################
echo -e "${YELLOW}2. Installing Flatpak bundle (release preferred)...${NC}"

DOWNLOAD_DIR=$(mktemp -d)
FLATPAK_BUNDLE="$DOWNLOAD_DIR/balatro-mod-manager.flatpak"
RELEASE_URL=""
RELEASE_JSON=""
if RELEASE_JSON=$(curl -fsSL "https://api.github.com/repos/rursache/balatro-mod-manager/releases" 2>/dev/null); then
    RELEASE_URL=$(printf '%s' "$RELEASE_JSON" | \
        grep -Eo '"browser_download_url":\s*"[^"]+\.flatpak"' | \
        head -n1 | cut -d '"' -f4 || true)
fi

if [[ -n "$RELEASE_URL" ]]; then
    echo -e "${YELLOW}Downloading latest release Flatpak...${NC}"
    if curl -fsSL "$RELEASE_URL" -o "$FLATPAK_BUNDLE"; then
        ensure_flathub_remote
        REQUIRED_RELEASE_RUNTIMES=(
            "org.gnome.Platform//50"
        )
        MISSING_RELEASE_RUNTIMES=()
        for runtime in "${REQUIRED_RELEASE_RUNTIMES[@]}"; do
            if ! flatpak info --user "$runtime" >/dev/null 2>&1; then
                MISSING_RELEASE_RUNTIMES+=("$runtime")
            fi
        done
        if [[ "${#MISSING_RELEASE_RUNTIMES[@]}" -gt 0 ]]; then
            echo -e "${YELLOW}Installing Flatpak runtimes required by release...${NC}"
            for runtime in "${MISSING_RELEASE_RUNTIMES[@]}"; do
                if ! flatpak remote-info --user flathub "$runtime" >/dev/null 2>&1; then
                    echo -e "${RED}Required runtime not found on Flathub: ${runtime}${NC}"
                    echo -e "${YELLOW}Attempting to resolve...${NC}"
                    echo -e "${YELLOW}Please try one of the following:${NC}"
                    echo "  1. Update Flatpak: sudo flatpak update"
                    echo "  2. Update remotes: flatpak update --appstream"
                    echo "  3. Reinstall flathub remote:"
                    echo "     flatpak remote-delete flathub"
                    echo "     flatpak remote-add --user --if-not-exists flathub https://flathub.org/repo/flathub.flatpakrepo"
                    echo ""
                    echo -e "${YELLOW}After updating, please retry this installation script.${NC}"
                    exit 1
                fi
            done
            if ! flatpak install --user -y flathub "${MISSING_RELEASE_RUNTIMES[@]}"; then
                echo -e "${RED}Failed to install required runtimes.${NC}"
                echo -e "${YELLOW}Please update your Flatpak installation and try again.${NC}"
                exit 1
            fi
        fi
        echo -e "${YELLOW}Installing Flatpak bundle...${NC}"
        flatpak install --user -y --reinstall "$FLATPAK_BUNDLE"
        cleanup 0
        echo -e "${GREEN}"
        echo "--------------------------------------"
        echo "Installation complete!"
        echo "--------------------------------------"
        echo -e "${NC}"

        echo "You can now launch the app via:"
        echo "  • Terminal: flatpak run io.balatro.ModManager"
        echo
        echo "Flatpak bundle downloaded from:"
        echo "  $RELEASE_URL"
        echo
        exit 0
    fi
fi

echo -e "${YELLOW}Release download failed; building Flatpak locally...${NC}"

# Require flatpak-builder for local build fallback
if ! command -v flatpak-builder &>/dev/null; then
    echo -e "${RED}flatpak-builder not found. Please install flatpak-builder and try again.${NC}"
    exit 1
fi
echo -e "${GREEN}flatpak-builder ✓${NC}"

RUNTIMES=(
    "org.gnome.Platform//50"
    "org.gnome.Sdk//50"
    "org.freedesktop.Sdk.Extension.node20//25.08"
    "org.freedesktop.Sdk.Extension.rust-stable//25.08"
)

MISSING_RUNTIMES=()
for runtime in "${RUNTIMES[@]}"; do
    if ! flatpak info --user "$runtime" >/dev/null 2>&1; then
        MISSING_RUNTIMES+=("$runtime")
    fi
done

if [[ "${#MISSING_RUNTIMES[@]}" -gt 0 ]]; then
    echo -e "${YELLOW}Installing Flatpak runtimes (first-time setup)...${NC}"
    ensure_flathub_remote
    if ! flatpak install --user -y flathub "${MISSING_RUNTIMES[@]}"; then
        echo -e "${RED}Failed to install required runtimes for local build.${NC}"
        echo -e "${YELLOW}Please try updating Flatpak and your remotes:${NC}"
        echo "  flatpak update --appstream"
        echo "  sudo flatpak update"
        echo ""
        echo -e "${YELLOW}Then retry this installation script.${NC}"
        exit 1
    fi
fi

flatpak-builder --force-clean --repo=repo build-dir packaging/flatpak/io.balatro.ModManager.json

FLATPAK_BUNDLE="$REPO_DIR/balatro-mod-manager.flatpak"
if [[ -e "$FLATPAK_BUNDLE" ]]; then
    PREEXIST_FLATPAK=true
fi

flatpak build-bundle repo "$FLATPAK_BUNDLE" io.balatro.ModManager master

echo -e "${YELLOW}3. Installing Flatpak bundle...${NC}"
flatpak install --user -y --reinstall "$FLATPAK_BUNDLE"

############################################
# CLEANUP (IF WE CLONED)
############################################
cleanup 0

echo -e "${GREEN}"
echo "--------------------------------------"
echo "Installation complete!"
echo "--------------------------------------"
echo -e "${NC}"

echo "You can now launch the app via:"
echo "  • Terminal: flatpak run io.balatro.ModManager"
echo
echo "Flatpak bundle saved at:"
echo "  $FLATPAK_BUNDLE"
echo
echo "Build artifacts:"
echo "  build-dir/ (Flatpak build dir)"
echo "  repo/ (Flatpak repo)"
echo

exit 0
