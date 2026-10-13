#!/bin/bash
set -e

SCRIPT_DIR="$(dirname "$0")"
cd "$SCRIPT_DIR/../aur-package"

# Get version from argument or Cargo.toml
if [ -n "$1" ]; then
    NEW_VERSION="$1"
else
    NEW_VERSION=$(grep '^version' ../src-tauri/Cargo.toml | head -1 | sed 's/.*"\(.*\)"/\1/')
fi

CURRENT_VERSION=$(grep '^pkgver=' PKGBUILD | cut -d= -f2)

echo "Current version: $CURRENT_VERSION"
echo "New version: $NEW_VERSION"

if [ "$CURRENT_VERSION" = "$NEW_VERSION" ]; then
    echo "Version unchanged. Nothing to do."
    exit 0
fi

# Backup original files
cp PKGBUILD PKGBUILD.bak
cp .SRCINFO .SRCINFO.bak

cleanup() {
    if [ -f PKGBUILD.bak ]; then
        mv PKGBUILD.bak PKGBUILD
        mv .SRCINFO.bak .SRCINFO
        echo "Reverted changes."
    fi
    rm -rf pkg src *.pkg.tar.zst *.deb
}

trap cleanup ERR

# Check if release exists before making changes
DEB_URL="https://github.com/rursache/balatro-mod-manager/releases/download/v${NEW_VERSION}/Balatro.Mod.Manager_${NEW_VERSION}_amd64.deb"
echo "Checking if release v$NEW_VERSION exists..."
HTTP_STATUS=$(curl -sI -o /dev/null -w "%{http_code}" "$DEB_URL")
if [ "$HTTP_STATUS" != "200" ] && [ "$HTTP_STATUS" != "302" ]; then
    echo "Error: Release v$NEW_VERSION not found (HTTP $HTTP_STATUS)"
    echo "Make sure you've published the GitHub release first."
    rm -f PKGBUILD.bak .SRCINFO.bak
    exit 1
fi

# Update PKGBUILD
sed -i "s/pkgver=.*/pkgver=$NEW_VERSION/" PKGBUILD

# Update checksum
echo "Downloading release and generating checksum..."
CHECKSUM=$(curl -sL "$DEB_URL" | sha256sum | cut -d' ' -f1)
if [ -z "$CHECKSUM" ] || [ "$CHECKSUM" = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855" ]; then
    echo "Error: Failed to download .deb or file is empty"
    exit 1
fi
sed -i "s/sha256sums=.*/sha256sums=('$CHECKSUM')/" PKGBUILD

# Regenerate .SRCINFO
makepkg --printsrcinfo > .SRCINFO

echo ""
echo "Updated to $NEW_VERSION"
echo ""

# Ask to test
read -p "Test build? [Y/n] " -n 1 -r
echo
if [[ ! $REPLY =~ ^[Nn]$ ]]; then
    rm -rf pkg src *.pkg.tar.zst
    if ! makepkg -sf; then
        echo "Build failed!"
        exit 1
    fi
    echo ""
    echo "Package built. Install with:"
    echo "  sudo pacman -U balatro-mod-manager-bin-$NEW_VERSION-1-x86_64.pkg.tar.zst"
    echo ""
fi

# Remove backups on success
rm -f PKGBUILD.bak .SRCINFO.bak
trap - ERR

# Ask to push
read -p "Commit and push to AUR? [y/N] " -n 1 -r
echo
if [[ $REPLY =~ ^[Yy]$ ]]; then
    rm -rf pkg src *.pkg.tar.zst *.deb
    git add PKGBUILD .SRCINFO
    git commit -m "Update to $NEW_VERSION"
    git push
    echo "Pushed to AUR!"
fi
