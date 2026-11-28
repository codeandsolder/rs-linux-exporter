#!/usr/bin/env bash
set -euo pipefail

# Run natively on the intended package distribution (CI uses that container).
cd "$(dirname "$0")"
# shellcheck source=/etc/os-release
source /etc/os-release
case "$ID:$VERSION_ID" in
  ubuntu:22.04) distro=ubuntu2204 ;;
  ubuntu:24.04) distro=ubuntu2404 ;;
  ubuntu:26.04) distro=ubuntu2604 ;;
  debian:12) distro=debian12 ;;
  debian:13) distro=debian13 ;;
  *) echo "Unsupported package distribution: $ID:$VERSION_ID" >&2; exit 1 ;;
esac
version=$(awk -F'"' '/^version =/ {print $2; exit}' Cargo.toml)
test -n "$version"
arch=$(dpkg --print-architecture)
root=$(mktemp -d)
trap 'rm -rf "$root"' EXIT
pkg="$root/pkg"
target_dir=$(realpath -m "${CARGO_TARGET_DIR:-target}")
cargo build --release --locked --target-dir "$target_dir"
bash packaging/check-binary-linkage.sh "$target_dir/release/rs-linux-exporter"
install -d "$pkg/DEBIAN" "$pkg/usr/bin" "$pkg/etc/rs-linux-exporter" "$pkg/lib/systemd/system" "$root/debian" dist
install -m 0755 "$target_dir/release/rs-linux-exporter" "$pkg/usr/bin/rs-linux-exporter"
install -m 0640 packaging/config.toml "$pkg/etc/rs-linux-exporter/config.toml"
install -m 0644 packaging/rs-linux-exporter.service "$pkg/lib/systemd/system/rs-linux-exporter.service"
install -m 0755 packaging/postinst packaging/prerm packaging/postrm "$pkg/DEBIAN/"
printf '/etc/rs-linux-exporter/config.toml\n' > "$pkg/DEBIAN/conffiles"
cat > "$root/debian/control" <<'CONTROL'
Source: rs-linux-exporter
Section: utils
Priority: optional
Maintainer: rs-linux-exporter contributors <noreply@example.com>
Standards-Version: 4.6.0

Package: rs-linux-exporter
Architecture: any
Description: Prometheus-compatible Linux metrics exporter written in Rust.
CONTROL
depends=$(cd "$root" && dpkg-shlibdeps -O -e"$pkg/usr/bin/rs-linux-exporter" | sed -n 's/^shlibs:Depends=//p')
test -n "$depends"
cat > "$pkg/DEBIAN/control" <<CONTROL
Package: rs-linux-exporter
Version: $version
Section: utils
Priority: optional
Architecture: $arch
Maintainer: rs-linux-exporter contributors <noreply@example.com>
Description: Prometheus-compatible Linux metrics exporter written in Rust.
Depends: $depends
CONTROL
dpkg-deb --root-owner-group --build "$pkg" "dist/rs-linux-exporter_${version}_${arch}_${distro}.deb"
