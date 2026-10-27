#!/bin/sh
# Create the node identity on first start, then hand over to `refrax`.
set -eu
if [ "${1:-}" = "node" ] && [ ! -f "${REFRAX_KEY}" ]; then
  refrax keys new --out "${REFRAX_KEY}"
fi
exec refrax "$@"
