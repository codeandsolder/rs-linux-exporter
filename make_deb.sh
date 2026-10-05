#!/usr/bin/env bash
set -euo pipefail

if [[ $# -lt 1 ]]; then
  echo "Usage: $0 <ubuntu2204|ubuntu2404|ubuntu2604|debian12|debian13> [more targets...]" >&2
  exit 1
fi

version=$(awk -F'"' '/^version =/ {print $2; exit}' Cargo.toml)
if [[ -z "${version}" ]]; then
  echo "Failed to read version from Cargo.toml" >&2
  exit 1
fi

if [[ ! -f Dockerfile.deb ]]; then
  echo "Missing Dockerfile.deb" >&2
  exit 1
fi

mkdir -p dist

for target in "$@"; do
  case "${target}" in
    ubuntu2204) base_image="ubuntu:22.04" ;;
    ubuntu2404) base_image="ubuntu:24.04" ;;
    ubuntu2604) base_image="ubuntu:26.04" ;;
    debian12) base_image="debian:12" ;;
    debian13) base_image="debian:13" ;;
    *)
      echo "Unknown target: ${target}" >&2
      exit 1
      ;;
  esac

  image_tag="rs-linux-exporter-deb-${target}"

  docker build \
    --build-arg BASE_IMAGE="${base_image}" \
    --build-arg VERSION="${version}" \
    --build-arg DIST="${target}" \
    -f Dockerfile.deb \
    -t "${image_tag}" \
    .

  container_id=$(docker create "${image_tag}")
  docker cp "${container_id}:/out/." dist/
  docker rm "${container_id}" >/dev/null
done
