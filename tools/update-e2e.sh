#!/usr/bin/env bash
# Prueba de punta a punta del hito U (plan-update.md U8) con un feed local
# firmado con la clave de prueba: instalar → firma mala (no prepara nada) →
# preparar 99.0.0 → "Actualizar ahora" (el guardián coloca y relanza) →
# desinstalar. Todo en una carpeta temporal y con WM_INSTANCE=utest: no toca
# la instalación, el registro ni el inicio automático de la usuaria (solo
# claves y accesos con sufijo "-utest", que se borran al final).
#
# Uso (Git Bash): tools/update-e2e.sh <carpeta con wm.exe y wmw.exe>
# Los binarios tienen que estar compilados con
#   WM_UPDATE_PUBKEY=8fd2ee4c…4408 (tests/update/README.md).
# Sale con 1 si algo no da lo esperado.
set -u
REPO=$(cd "$(dirname "$0")/.." && pwd)
BIN=$(cd "${1:?carpeta de los binarios}" && pwd)
KEY=$REPO/tests/update/test-key.pem
FAIL=0
check() { if eval "$2"; then echo "  ok: $1"; else echo "  FALLA: $1"; FAIL=1; fi; }
T=$(cygpath -u "$TEMP")/wm-u-test
rm -rf "$T"; mkdir -p "$T/feed1" "$T/feed2" "$T/install"
export WM_INSTANCE=utest
export WM_INSTALL_ROOT=$(cygpath -w "$T/install")
export WM_UPDATE=1

sign() { openssl dgst -sha256 -sign "$KEY" -out "$1.sig" "$1"; }

# Feed 1: la versión actual. wm-settings.exe puede ser un relleno (no se
# ejecuta en la prueba).
mkdir -p "$T/src"; cp "$BIN/wm.exe" "$BIN/wmw.exe" "$T/src/"
if [ -f "$BIN/wm-settings.exe" ]; then cp "$BIN/wm-settings.exe" "$T/src/"; else cp "$BIN/wm.exe" "$T/src/wm-settings.exe"; fi
"$BIN/wm.exe" release-pack "$(cygpath -w "$T/src")" --arch x64 --notes "Primera" --out "$(cygpath -w "$T/feed1")" >/dev/null
sign "$T/feed1/latest-x64.txt"

# Feed 2: "99.0.0": los mismos exe con bytes agregados al final.
for f in wmw wm wm-settings; do
  cp "$T/src/$f.exe" "$T/feed2/$f-x64.exe"; printf 'version 99' >> "$T/feed2/$f-x64.exe"
done
{
  echo "wm-update 1"; echo "version = 99.0.0"; echo "arch = x64"; echo "notes = Prueba"
  for f in wmw wm wm-settings; do
    size=$(stat -c %s "$T/feed2/$f-x64.exe"); sha=$(sha256sum "$T/feed2/$f-x64.exe" | cut -d' ' -f1)
    echo "file = $f.exe | $f-x64.exe | $size | $sha"
  done
} > "$T/feed2/latest-x64.txt"
sign "$T/feed2/latest-x64.txt"

echo "== 1. instalar"
WM_UPDATE_FEED=$(cygpath -w "$T/feed1") "$BIN/wm.exe" setup --quiet
sleep 3
check "archivos instalados" '[ -f "$T/install/wmw.exe" ] && [ -f "$T/install/wm.exe" ] && [ -f "$T/install/wm-settings.exe" ]'
check "entrada en Aplicaciones instaladas" 'reg query "HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\wm-utest" //v DisplayVersion >/dev/null 2>&1'
check "acceso directo" '[ -f "$APPDATA/Microsoft/Windows/Start Menu/Programs/wm-utest.lnk" ]'
check "wm arrancó (guardián y core)" '[ "$(tasklist //FI "IMAGENAME eq wmw.exe" | grep -c wmw.exe)" -ge 2 ]'

echo "== 2. feed con firma mala: no prepara nada"
cp -r "$T/feed2" "$T/feedbad"; printf 'x' >> "$T/feedbad/latest-x64.txt"
WM_UPDATE_FEED=$(cygpath -w "$T/feedbad") "$T/install/wm.exe" update run --force
check "firma mala: no preparó nada" '[ ! -d "$T/install/update/99.0.0" ]'

echo "== 3. preparar 99.0.0"
WM_UPDATE_FEED=$(cygpath -w "$T/feed2") "$T/install/wm.exe" update run --force
check "99.0.0 lista" '[ -f "$T/install/update/99.0.0/ready" ]'

echo "== 4. actualizar ahora"
"$T/install/wm.exe" actualizar
sleep 5
for f in wmw wm wm-settings; do
  check "$f.exe es el de 99.0.0" '[ "$(sha256sum "$T/install/$f.exe" | cut -d" " -f1)" = "$(sha256sum "$T/feed2/$f-x64.exe" | cut -d" " -f1)" ]'
done
check "la versión anterior quedó en .old" '[ -f "$T/install/wmw.exe.old" ]'
check "wm volvió a arrancar" '[ "$(tasklist //FI "IMAGENAME eq wmw.exe" | grep -c wmw.exe)" -ge 2 ]'

echo "== 5. desinstalar"
"$T/install/wm.exe" uninstall --quiet
sleep 1
check "carpeta borrada" '[ ! -d "$T/install" ]'
check "sin entrada en Aplicaciones instaladas" '! reg query "HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\wm-utest" >/dev/null 2>&1'
check "sin acceso directo" '[ ! -f "$APPDATA/Microsoft/Windows/Start Menu/Programs/wm-utest.lnk" ]'
check "wm cerrado" '[ "$(tasklist //FI "IMAGENAME eq wmw.exe" | grep -c wmw.exe)" -eq 0 ]'
rm -rf "$T"
if [ $FAIL -eq 0 ]; then echo "update-e2e: todo bien"; else echo "update-e2e: CON FALLAS"; fi
exit $FAIL
