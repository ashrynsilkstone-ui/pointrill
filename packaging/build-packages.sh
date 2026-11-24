#!/usr/bin/env bash
set -euo pipefail

if [[ $# != 2 ]]; then
    printf 'Usage: bash packaging/build-packages.sh PORTABLE_BINARY OUTPUT_DIRECTORY\n' >&2
    exit 2
fi

source_dir=$(cd "$(dirname "$0")/.." && pwd)
binary=$(realpath "$1")
mkdir -p "$2"
output=$(realpath "$2")
version=$(sed -n 's/^version = "\([^"]*\)"$/\1/p' "$source_dir/Cargo.toml")
package_dir="$source_dir/packaging"
app_id=io.github.ashrynsilkstone_ui.Pointrill
deb_tool=${DPKG_DEB:-dpkg-deb}

export LC_ALL=C
readelf -h "$binary" | awk '/Machine:/ && /X86-64/ { found = 1 } END { exit !found }'
glibc=$(readelf --version-info "$binary" |
    awk '{ for (i = 1; i < NF; i++) if ($i == "Name:" && $(i+1) ~ /^GLIBC_[0-9]/) print substr($(i+1), 7) }' |
    sort -V | tail -n 1)
if [[ -z "$glibc" || $(printf '%s\n' 2.34 "$glibc" | sort -V | tail -n 1) != 2.34 ]]; then
    printf 'Pointrill needs an x86-64 binary linked against glibc 2.34 or older.\n' >&2
    exit 1
fi

work_dir=$(mktemp -d)
trap 'rm -rf -- "$work_dir"' EXIT
payload="$work_dir/payload"
install -Dm755 "$binary" "$payload/usr/bin/pointrill"
install -Dm644 "$package_dir/$app_id.desktop" "$payload/usr/share/applications/$app_id.desktop"
install -Dm644 "$package_dir/$app_id.Tray.desktop" "$payload/etc/xdg/autostart/$app_id.Tray.desktop"
doc_dir="$payload/usr/share/doc/pointrill"
install -Dm644 -t "$doc_dir" \
    "$source_dir/README.md" "$source_dir/README.de.md" "$package_dir/69-pointrill.rules" \
    "$source_dir/assets/fonts/barlow/OFL.txt"
install -Dm644 -t "$doc_dir/docs/screenshots" "$source_dir/docs/screenshots/"*.png

install -m644 "$package_dir/PKGBUILD" "$work_dir/"
(
    cd "$work_dir"
    POINTRILL_VERSION=$version PKGDEST=$work_dir PKGEXT=.pkg.tar.zst \
        makepkg --nodeps --noconfirm --noprogressbar
)

rpm_dir="$work_dir/rpm"
mkdir -p "$rpm_dir/SOURCES" "$rpm_dir/tmp"
tar -cf "$rpm_dir/SOURCES/pointrill-payload.tar" -C "$payload" .
rpmbuild -bb --nodeps --define "_topdir $rpm_dir" --define "_tmppath $rpm_dir/tmp" \
    --define "pointrill_version $version" \
    "$package_dir/pointrill.spec"

install -d "$payload/DEBIAN"
{
    printf 'Version: %s-1\n' "$version"
    printf 'Installed-Size: %s\n' "$(du -sk "$payload" | cut -f1)"
    cat "$package_dir/deb-control"
} > "$payload/DEBIAN/control"
printf '/etc/xdg/autostart/%s.Tray.desktop\n' "$app_id" > "$payload/DEBIAN/conffiles"
deb="pointrill_${version}-1_amd64.deb"
"$deb_tool" --root-owner-group --build "$payload" "$work_dir/$deb"

arch="pointrill-$version-1-x86_64.pkg.tar.zst"
rpm="pointrill-$version-1.x86_64.rpm"
install -m644 -t "$output" "$work_dir/$arch" "$rpm_dir/RPMS/x86_64/$rpm" "$work_dir/$deb"
(
    cd "$output"
    sha256sum "$arch" "$rpm" "$deb" > SHA256SUMS
)
