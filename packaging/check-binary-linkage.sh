#!/usr/bin/env bash
set -euo pipefail

# The shared BTLS provider and its C++ runtime must travel with the binary.
# The GNU/Linux release and distribution packages may still depend on libc.
needed=$(readelf -d "$1")
if [[ "$needed" =~ \[(libstdc\+\+|libc\+\+|libc\+\+abi|libssl|libcrypto)\.so([.][^]]*)?\] ]]; then
    printf 'Unexpected dynamic TLS or C++ runtime dependency: %s\n' "${BASH_REMATCH[0]}" >&2
    exit 1
fi
