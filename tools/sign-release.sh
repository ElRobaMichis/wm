#!/usr/bin/env bash
# Firma los manifiestos de un release ya publicado por la CI (release.yml) y
# sube las firmas (plan-update.md U7). Corre en la PC del autor: la clave
# privada (%USERPROFILE%\.wm-release\wm-update-key.pem) nunca sale de ahí.
#
# Antes de firmar, baja cada asset y comprueba que su tamaño y SHA-256 sean
# los del manifiesto: se firma solo lo que se verificó.
#
# Uso (Git Bash): tools/sign-release.sh v0.1.0
set -euo pipefail
TAG=${1:?tag, por ejemplo v0.1.0}
REPO_GH=ElRobaMichis/wm
ROOT=$(cd "$(dirname "$0")/.." && pwd)
KEY=${WM_UPDATE_KEY_FILE:-$USERPROFILE/.wm-release/wm-update-key.pem}
[ -f "$KEY" ] || { echo "No está la clave privada en $KEY"; exit 1; }

# La clave tiene que ser la de update-pubkey.txt (la que llevan los binarios).
pub=$(openssl ec -in "$KEY" -pubout -outform DER 2>/dev/null | tail -c 64 | xxd -p | tr -d '\n')
if [ "$pub" != "$(tr -d ' \r\n' < "$ROOT/update-pubkey.txt")" ]; then
  echo "La clave no coincide con update-pubkey.txt."; exit 1
fi

W=$(mktemp -d)
trap 'rm -rf "$W"' EXIT
gh release download "$TAG" --repo "$REPO_GH" --dir "$W" --pattern 'latest-*.txt'
for m in "$W"/latest-*.txt; do
  echo "== $(basename "$m")"
  grep -E '^(version|arch) ' "$m"
  v=$(sed -n 's/^version = //p' "$m")
  [ "v$v" = "$TAG" ] || { echo "El manifiesto dice $v, no $TAG."; exit 1; }
  while IFS='|' read -r name asset size sha; do
    asset=$(echo "$asset" | xargs); size=$(echo "$size" | xargs); sha=$(echo "$sha" | xargs)
    gh release download "$TAG" --repo "$REPO_GH" --dir "$W" --pattern "$asset" --clobber
    got_size=$(stat -c %s "$W/$asset"); got_sha=$(sha256sum "$W/$asset" | cut -d' ' -f1)
    if [ "$got_size" != "$size" ] || [ "$got_sha" != "$sha" ]; then
      echo "  $asset NO coincide con el manifiesto."; exit 1
    fi
    echo "  ok $asset ($size bytes)"
  done < <(sed -n 's/^file = //p' "$m")
  openssl dgst -sha256 -sign "$KEY" -out "$m.sig" "$m"
  openssl ec -in "$KEY" -pubout 2>/dev/null > "$W/pub.pem"
  openssl dgst -sha256 -verify "$W/pub.pem" -signature "$m.sig" "$m"
done
gh release upload "$TAG" --repo "$REPO_GH" --clobber "$W"/latest-*.txt.sig
echo "Firmas subidas a $TAG."
