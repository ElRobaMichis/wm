# Instalar y actualizar sin terminal (hito U)

> **Estado (16/11/2026): U1–U7 implementados; U8 como `tools/update-e2e.sh` (en la CI, job `ventanas`, x64 y ARM64). Primer release `v0.1.0` publicado como pre-release y firmado con `tools/sign-release.sh`; `wm-setup-x64.exe` bajado de GitHub instaló, verificó la firma real y arrancó en una carpeta de prueba (3 s), y se desinstaló limpio.** Marcada como versión oficial (`latest`) a pedido del usuario: **los releases ya no salen como pre-release**; la compuerta es la firma (un release sin `.sig` es invisible para los instalados: el 404 de la firma cuenta como "al día"). Probado en esta PC con un feed local firmado con la clave de prueba: instalar → firma mala (no prepara nada) → preparar 99.0.0 → "Actualizar ahora" (el guardián coloca y relanza con los escritorios) → desinstalar, 14/14. Contra GitHub real, la descarga por WinHTTP responde (404 hasta que haya release).
>
> **Desvíos del diseño:**
> - El instalador usa dos `MessageBox` en vez de `TaskDialog` (recorte previsto), sin barra de progreso y sin casilla: dice en el texto que se abre solo al prender la compu y que se puede cambiar desde su ícono.
> - Sin el ítem "Actualizar sola" en el menú (siempre activo; `WM_UPDATE=0` lo apaga).
> - La prueba de "versión rota → volver atrás" queda en las pruebas del modelo (`rollback_plan`) y a mano: una versión que no pasa `self-check` ni se prepara.
>
> **Cambio de U0/U7 (16/11/2026): la clave privada nunca se sube a GitHub.** Vive solo en la PC del autor (`%USERPROFILE%\.wm-release\wm-update-key.pem`; **hacer una copia**: sin ella, una versión nueva no llega y hay que reinstalar a mano). La CI compila y publica la pre-release **sin firmas**; `tools/sign-release.sh vX.Y.Z`, en esa PC, baja los assets, comprueba tamaño y SHA-256 contra el manifiesto, firma y sube los `.sig`. Sin `.sig`, ningún wm instalado acepta la versión. Así no hacen falta el environment ni el secret, y un robo de la cuenta de GitHub no alcanza para firmar.

Diseño de Fable 5.2 (16/11/2026), pedido por el usuario: que la usuaria de prueba tenga `wm` instalado y que, cuando él publique una versión en GitHub, le llegue sola, sin descargar de nuevo ni tocar la terminal. **[V]** = verificado en el código; **[S]** = supuesto a comprobar.

## Decisiones

- **Qué baja ella:** un solo `wm-setup-x64.exe`, con los mismos bytes que `wmw.exe`. Si el ejecutable se llama `wm-setup…`, entra en modo instalador: una pantalla, baja los tres ejecutables con el código del actualizador, verifica, registra y arranca. Instalar es "actualizar desde cero"; volver a correrlo repara.
- **Actualizar:** se descarga sola en segundo plano y se aplica sola al próximo arranque de `wm`, o cuando ella toca el globo o "Actualizar ahora". **Nunca reinicia sola en medio del uso.**
- **La red vive fuera del core:** rol efímero `wmw.exe update run`, con prioridad de fondo. El proceso que ve las teclas nunca abre un socket; WinHTTP no se carga en el guardián ni en el core.
- **Firma del manifiesto:** ECDSA P-256 + SHA-256 con CNG (`bcrypt.dll` por `dynlib`), cero crates nuevos. Se firma con `openssl`; cualquiera verifica con `openssl dgst -verify`.
- **Fuente:** archivo fijo `https://github.com/ElRobaMichis/wm/releases/latest/download/latest-<arch>.txt` más `.sig`. Sin la API de GitHub (sin JSON ni límite de 60 pedidos por hora); "latest" ignora las prereleases, que sirven de compuerta.
- **Reemplazo:** rutas estables y el exe en uso se renombra a `.old` (el ícono fijado en la bandeja depende de la ruta [S]).
- **Vuelta atrás:** el guardián viejo lanza el core nuevo y lo vigila 60 s; si no arranca o se rinde, renombra de vuelta y sigue con la versión anterior.
- **Versión:** `[workspace.package] version` → `env!("CARGO_PKG_VERSION")` [V].
- **Temporizador:** uno de 24 h en el hilo `wm-tray` (despierta una vez por día; lo prohibido son los periódicos cortos).
- **x64 y ARM64**, con manifiestos independientes.
- **Descartados:** Inno Setup, Velopack, `self_update` (dependencias o toolchains pesados, sin integración con el guardián); `curl.exe` y PowerShell (patrón de malware para Defender).

## Formatos

Manifiesto (`clave = valor`, como `apps.txt`; las claves desconocidas se ignoran):

```
wm-update 1
version = 0.2.0
arch = x64
notes = Arregla Z con ventanas maximizadas.
file = wmw.exe | wmw-x64.exe | 1482752 | <sha256 hex>
file = wm.exe | wm-x64.exe | … | …
file = wm-settings.exe | wm-settings-x64.exe | … | …
```

Disco:

```
%LOCALAPPDATA%\Programs\wm\
  wmw.exe  wm.exe  wm-settings.exe     rutas estables (Run, acceso directo, bandeja)
  *.old                                versión anterior, hasta confirmar
  update\<versión>\                    3 exe + latest.txt + .sig + ready (se escribe al final)
  update\journal                       "applying 0.1.0 0.2.0" | "swapped 0.1.0 0.2.0"
%LOCALAPPDATA%\wm\update.txt           auto=1, last_check, bad=, notified=
%APPDATA%\wm\apps.txt                  nunca se toca
```

Señales: código de salida 10 del core = "aplicar y relanzar"; `WM_UPDATED_FROM` / `WM_UPDATE_FAILED` del guardián al core (para el globo); `WM_UPDATE=0` lo apaga; `WM_UPDATE_FEED=<carpeta>` feed local para pruebas (igual exige firma); `WM_INSTALL_ROOT` carpeta de instalación para pruebas.

## Pasos

- **U0. Llaves y versión.** Clave ECDSA P-256 (`openssl ecparam -name prime256v1 -genkey -noout -out wm-update-key.pem`), con copia fuera de línea: si se pierde, ella tiene que reinstalar a mano. Environment `release` en GitHub, restringido a tags `v*`, con el secret `WM_UPDATE_KEY`. **Lo hace el usuario** (es una credencial). Versión `0.1.0`.
- **U1. Núcleo puro** (`wm-model/src/update.rs`): `Version`, `parse_manifest` (≤ 16 KB, nombres de una lista fija, assets `[a-z0-9.-]+`), `der_to_raw` (firma DER de openssl → r‖s), `decide` (solo versión mayor, misma arquitectura, no marcada `bad`), `State`, `swap_plan` / `recover` con propiedad "cortar en cualquier paso deja una sola versión".
- **U2. Plataforma** (`sign.rs`, `net.rs`, por `dynlib`): SHA-256 y verificación P-256 con CNG; `get(url, max, plazo)` con WinHTTP (solo `https://`, nunca https→http, corta al pasar el máximo, User-Agent `wm` sin versión). La CI prohíbe `winhttp`, `bcrypt.dll`, `urlmon`, `wininet` estáticos.
- **U3. Buscar y preparar** (`app/src/update.rs`): `wm update check|run [--force] [--tag]|status`, `wm self-check`, `wm release-pack`. `run`: mutex `Local\wm.update` → límite de 20 h → manifiesto + firma → `decide` → bajar a `update\<v>\*.part` desde un host fijo (nunca URLs del manifiesto) → tamaño + SHA-256 → `self-check` (5 s) → `ready` → avisar a la bandeja. Cualquier falla borra los `.part`. Sin red: solo una línea en el log. Activo solo en la copia instalada.
- **U4. Guardián:** al arrancar, `recover`, limpiar `*.old`, `apply_staged` si hay `ready` (re-verifica, journal, renombres con `wmw.exe` al final); prueba de 60 s con `WM_UPDATED_FROM`; rendirse en la prueba → `rollback`, `bad=<v>`, `WM_UPDATE_FAILED`. Código 10: aplicar y relanzar con `WM_RELAUNCH=1` (D4: conserva los escritorios); el core sale con `forget_desks = false`.
- **U5. Instalar y desinstalar** (`install.rs`, `setup.rs`): carpeta, clave `HKCU\…\Uninstall\wm`, acceso directo en el menú Inicio (`IShellLinkW`), "Abrir wm cuando prendo la compu" (marcado). `wm uninstall`: Salir, `restore_all`, quitar Run, acceso y clave, borrar la carpeta y `%LOCALAPPDATA%\wm`; `%APPDATA%\wm` se conserva (`--purge` lo borra).
- **U6. Bandeja:** órdenes `UpdateReady`, `UpdateNow` (`wm actualizar`), `UpdateCheck`; timer 3 min después de arrancar, luego 24 h, y 2 min tras volver del reposo; globo "Hay una versión nueva de wm · Tocá acá para actualizar"; menú "Actualizar ahora (0.2.0)", "Actualizar sola" ✓, "Versión 0.1.0 · buscar actualizaciones"; globos "wm se actualizó" y "No se pudo actualizar wm · Seguís con la versión de antes".
- **U7. Publicar** (`release.yml`, tag `v*`): compilar x64 y ARM64, `release-pack`, firmar los manifiestos con el secret, `gh release create --prerelease`; prueba de humo instalando desde el release real. **Le llega a ella cuando el usuario desmarca "pre-release".**
- **U8. `ci-probe`:** instalar → feed con un byte cambiado, firma mala o versión menor (no prepara nada) → actualizar con los títeres en varios escritorios → feed roto (rollback, `bad`) → desinstalar. Feeds firmados con una clave de prueba pública; el binario de release no la acepta.
- **U9. Documentación:** este archivo, architecture.md §12.4, README "Instalar".

## Seguridad: invariantes

- Solo se ejecuta lo que pasó firma (clave embebida) + tamaño + SHA-256, y solo después de colocarlo (excepción: `self-check`, ya verificado).
- Nada baja a `%TEMP%` ni se ejecuta desde ahí; nombres de una lista fija; host fijo; nunca se baja de versión.
- `ready` se escribe al final: una descarga a medias es invisible.
- Siempre hay un guardián que funciona, dueño de la vuelta atrás.
- Privacidad: solo `github.com` y su CDN; sin identificadores ni versión en el User-Agent.

## A mano

- En su PC, primero: **Control inteligente de aplicaciones**. Si está "Activado", bloquea cualquier exe sin firmar sin botón para saltearlo [S].
- Lo que va a ver una sola vez: Edge "no se descarga habitualmente" → ⋯ → Conservar → Conservar de todos modos; después "Windows protegió su PC" → Más información → Ejecutar de todas formas.
- Las actualizaciones no repiten el aviso (los archivos que escribe el actualizador no llevan marca de internet [S]).
- Defender en una VM limpia y en su PC; ciclo real de 24 h; cortar la red a mitad; portal cautivo; desinstalar desde Configuración; el ícono fijado tras actualizar.

## Recortes

**Fuera de esta vez:** firma Authenticode, deltas, canales, barra de progreso real, winget/scoop, ícono propio del acceso directo. **Si falta tiempo:** TaskDialog → dos `MessageBox`; release ARM64; "buscar actualizaciones"; clic en el globo. **No recortar:** firma, `self-check`, vuelta atrás, chequeo del `ci-probe`.

**Recién se le entrega con U1–U7 completos:** una versión sin actualizador la obligaría a bajar de nuevo.
