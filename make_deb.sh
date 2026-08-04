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

mkdir -p dist

for target in "$@"; do
  case "${target}" in
    ubuntu2204|ubuntu2404|ubuntu2604|debian12|debian13)
      dockerfile="Dockerfile.${target}"
      ;;
    *)
      echo "Unknown target: ${target}" >&2
      exit 1
      ;;
  esac

  if [[ ! -f "${dockerfile}" ]]; then
    echo "Missing ${dockerfile}" >&2
    exit 1
  fi

  image_tag="rs-linux-exporter-deb-${target}"

  docker build \
    --build-arg VERSION="${version}" \
    --build-arg DIST="${target}" \
    -f "${dockerfile}" \
    -t "${image_tag}" \
    .

  container_id=$(docker create "${image_tag}")
  docker cp "${container_id}:/out/." dist/
  docker rm "${container_id}" >/dev/null

done
