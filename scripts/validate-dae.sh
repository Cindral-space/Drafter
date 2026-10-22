#!/usr/bin/env bash
set -euo pipefail
if ! command -v xmllint >/dev/null 2>&1; then
    echo 'SKIP: xmllint is not installed.'
    exit 0
fi
if [ "$#" -eq 0 ]; then
    echo 'Usage: validate-dae.sh FILE.dae [...]' >&2
    exit 2
fi
xmllint --nonet --noout "$@"
if [ -n "${COLLADA_141_SCHEMA:-}" ] && [ -f "$COLLADA_141_SCHEMA" ]; then
    xmllint --nonet --noout --schema "$COLLADA_141_SCHEMA" "$@"
else
    echo 'SKIP: COLLADA 1.4.1 schema validation; no offline COLLADA_141_SCHEMA available (XML well-formedness passed).'
fi
