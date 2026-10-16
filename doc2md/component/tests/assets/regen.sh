#!/usr/bin/env bash
# Regenerate the legacy binary Office fixtures (doc/xls/ppt) from the flat-ODF
# sources in this directory using headless LibreOffice in Docker. The resulting
# binaries are committed and consumed by scripts/gen_fixtures.py. RTF is generated
# directly in gen_fixtures.py and is not produced here.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
image="lscr.io/linuxserver/libreoffice:latest"
run() {
  docker run --rm -v "$here:/data" -w /data --entrypoint soffice "$image" \
    --headless --convert-to "$1" --outdir /data "/data/$2"
}
run "doc:MS Word 97" doc.fodt
run "xls:MS Excel 97" xls.fods
run "ppt:MS PowerPoint 97" ppt.fodp
echo "regenerated: doc.doc xls.xls ppt.ppt"
