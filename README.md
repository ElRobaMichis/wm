# wm (nombre provisorio)

Gestor de ventanas para Windows, escrito en Rust. Pensado para que cualquiera lo use sin leer documentación: pocos atajos lógicos, mouse como opción de primera clase, consumo mínimo y animaciones fluidas a la frecuencia real del monitor.

> **Estado:** en desarrollo, hitos 0 a 5 de la primera versión hechos ([plan-core.md](plan-core.md)): `Caps` + letra va a una app; `Caps` + Alt acomoda la ventana al frente (mitades, esquinas, otro monitor, maximizar, centrar, agrandar y achicar, deshacer) con animación; mantener `Caps` muestra el mapa de teclado; `Caps` + 1–4 cambia de escritorio; `Caps` + arrastrar mueve una ventana desde cualquier parte. Arranca con `wmw` (sin consola, con ícono en la bandeja) y se puede iniciar con Windows.

## Instalar (sin terminal)

1. Bajar `wm-setup-x64.exe` (o `wm-setup-arm64.exe`) del [último release](https://github.com/ElRobaMichis/wm/releases/latest) y abrirlo.
2. Windows avisa una sola vez, porque el ejecutable no está firmado: en Edge, ⋯ → Conservar → Conservar de todos modos; después, "Windows protegió su PC" → Más información → Ejecutar de todas formas.
3. "Instalar": se instala solo para el usuario (sin administrador) en `%LOCALAPPDATA%\Programs\wm`, con acceso directo en el menú Inicio, entrada en "Aplicaciones instaladas" y arranque con Windows.

**Actualizaciones:** la copia instalada se fija una vez por día en GitHub (sin mandar datos); si hay una versión nueva, la baja en segundo plano, verifica su firma y avisa con un globo. Se aplica al tocar el globo o "Actualizar ahora" en el menú, o sola en el próximo arranque; si la versión nueva no arranca, vuelve sola a la anterior. Detalle en [plan-update.md](plan-update.md).

**Desinstalar:** Configuración → Aplicaciones → wm, o `wm uninstall`. Las letras elegidas (`%APPDATA%\wm`) quedan por si se vuelve a instalar.

**Publicar una versión** (desarrollador): subir `version` en el `Cargo.toml`, `git tag -a vX.Y.Z -m "notas"` y `git push origin vX.Y.Z`. La CI crea el release sin firmas; después, en la PC que tiene la clave, `tools/sign-release.sh vX.Y.Z` verifica y firma. Recién firmado les llega a los instalados (sin firma, la ignoran).

## Probar

```bash
cargo build --release
target/release/wmw
```

`wmw.exe` es `wm` sin ventana de consola: arranca el guardián, que lanza el core y lo vuelve a lanzar si se cae (mostrando antes todas las ventanas que haya escondido). Aparece un ícono en la bandeja:

- muestra el número del escritorio activo;
- **clic (izquierdo o derecho):** los escritorios 1–4 (con cuántas ventanas tiene cada uno), "Abrir *app* siempre así" (la última ventana que tuviste al frente), Ajustes, Pausar atajos, Deshacer el último movimiento, Mostrar todas las ventanas, Iniciar con Windows y Salir.

`wm.exe` es el mismo programa con consola (para ver el log en vivo y para la línea de comandos): `wm` sin argumentos hace lo mismo que `wmw`; `wm core` corre solo el core.

| Teclas | Qué hace |
|---|---|
| `Caps` + letra | Va a la app con esa inicial (por ejemplo `Caps` + `B` = Brave), como un Alt+Tab de esa letra: la ventana más reciente, o la anterior si ya estás en una; sin soltar `Caps`, repetir la letra recorre las demás. Si la app está en la bandeja (Steam), la abre |
| `Caps` + Alt + W/A/S/D | Esa mitad; repetir = al otro monitor; dos juntas = esquina |
| `Caps` + Alt + F / C / Z / G / V | Maximizar / centrar / deshacer / agrandar / achicar |
| `Caps` + 1 / 2 / 3 / 4 | Ir a ese escritorio (las ventanas de los otros se esconden) |
| `Caps` + Alt + 1 / 2 / 3 / 4 | Llevar la ventana al frente a ese escritorio (Z lo deshace) |
| `Caps` + arrastrar | Mover la ventana agarrándola desde cualquier parte (Z lo deshace) |
| `Caps` + clic derecho + arrastrar | Cambiar el tamaño desde la esquina más cercana |
| Mantener `Caps` | El mapa de teclado (con Alt, la capa de ventana) |
| `Caps` (toque corto) | Bloq Mayús normal |

**Reglas por app (v1.1):** en los ajustes, cada letra con app puede tener una regla "Al abrir": no hacer nada, mitad izquierda, mitad derecha, maximizada o centrada (80 %). Se aplica una sola vez, a la primera ventana principal de la app, cuando la app termina de ubicarla (también al volver de la bandeja, como Steam); si después la app se mueve sola, gana la app, y `Caps` + Alt + Z la devuelve adonde la abrió. En la pestaña Opciones: **espacio entre ventanas** (sin espacio, 4, 8, 12, 16 o 24 px, contra los bordes y entre mitades y esquinas; maximizar y centrar no cambian) y, con más de un monitor, **abrir en la pantalla del puntero** (las apps con regla van a la pantalla donde está el puntero).

**v1.2:** una app vive en una sola letra (si la elegís para otra, se muda con su regla y los ajustes avisan qué apps abiertas pierden esa inicial). Además de "Al abrir", cada letra puede tener **"En el escritorio"**: una ventana nueva de la app va al escritorio 1–4 elegido y `wm` va con ella ("Brave se abre en el escritorio 2"; Z vuelve). **"Abrir siempre así"** (bandeja) guarda cómo está la ventana al frente (mitad, esquina, maximizada o centrada) como su regla "Al abrir". **Sugerir atajos** (prendido): si vas a una app con letra usando Alt+Tab o la barra de tareas, un aviso te recuerda la letra ("La próxima vez: `Caps` + B"; desde el tercer cambio, uno cada 10 minutos, nunca más tras 5 usos de esa letra o 5 avisos). **Registro de uso** (apagado): cuenta en `%LOCALAPPDATA%\wm\uso.txt` qué atajos usás por día, sin títulos ni nombres de app; `wm uso` muestra el resumen. El mapa, los avisos y la lista dibujan emoji, chino, coreano y símbolos con fuentes de Windows.

Al conectar o desconectar un monitor, cambiar la resolución o la escala, volver del reposo o desbloquear, `wm` espera a que Windows termine de acomodar y trae entera a la pantalla más cercana cualquier ventana que haya quedado afuera (también las de otros escritorios). "Mostrar todas las ventanas" hace lo mismo en el momento.

Línea de comandos (con `wm` corriendo): `wm quit`, `wm pause`, `wm resume`, `wm undo`, `wm ajustes`, `wm mostrar` (todas las ventanas al escritorio activo); `wm autostart on|off` (iniciar con Windows); `wm apps` muestra a qué app apunta cada letra; `wm abrir-asi` ("Abrir siempre así" con la ventana al frente); `wm uso` (el registro de uso); `wm restore` muestra cualquier ventana que haya quedado escondida; `wm fuera` dice si hay ventanas fuera de pantalla (solo mira). El log está en `%LOCALAPPDATA%\wm\wm.log`.

Sin configuración, cada letra va a la app abierta más reciente cuyo nombre de ejecutable empieza con esa letra. Para fijar letras, usar los ajustes o editar `%APPDATA%\wm\apps.txt` (se recarga solo al guardar):

```text
# letra = ejecutable (sin .exe) | ruta para abrirla si está cerrada | nombre | al abrir: regla | escritorio: N
b = brave | | | al abrir: derecha | escritorio: 2
t = windowsterminal
n = notion | C:\Users\yo\AppData\Local\Programs\Notion\Notion.exe | | al abrir: centrada 60
```

Reglas a mano: `izquierda`, `derecha`, `arriba`, `abajo`, las cuatro esquinas (`arriba-izquierda`…), `maximizada`, `centrada` o `centrada N` (30–100 %). `escritorio: N` va de 1 a 4. Las opciones están en `%APPDATA%\wm\settings.txt` (`espacio_entre_ventanas = 8`, `abrir_en_la_pantalla_del_puntero = sí`, `sugerir_atajos = no`, `registro_de_uso = sí`, …).

## Documentos

- [findings.md](findings.md): investigación de usuarios y mercado.
- [keyboard-design.md](keyboard-design.md): modelo de interacción y mapa de atajos ("Apps primero").
- [architecture.md](architecture.md): arquitectura técnica y plan de spikes.
- [design.md](design.md): lenguaje de diseño (principios, tokens y componentes).
- [plan-core.md](plan-core.md): plan de la primera versión (hitos 0–5), con [plan-hito4.md](plan-hito4.md), [plan-hito4d.md](plan-hito4d.md) (monitores) y [plan-hito5.md](plan-hito5.md).
- [plan-update.md](plan-update.md): instalar y actualizar sin terminal.
- [plan-apps.md](plan-apps.md): apps (bandeja, selector, el Alt+Tab de la letra, la lista de ventanas).
- [plan-v1.1.md](plan-v1.1.md): reglas por app, espacio entre ventanas y abrir en la pantalla del puntero.
- [plan-v1.2.md](plan-v1.2.md): escritorio al abrir, "abrir siempre así", una app por letra, sugerir atajos, registro de uso y respaldo de glifos.
- [funciones-ocultas.md](funciones-ocultas.md): lo que se escondió de los ajustes en la primera versión, por qué y qué falta para mostrarlo.

## Compilar

Requisitos: Rust estable (1.91 o posterior). El objetivo de release es MSVC (`x86_64-pc-windows-msvc` y `aarch64-pc-windows-msvc`); el toolchain GNU sirve para desarrollar.

```bash
cargo build --release
cargo test
target/release/wm doctor
```

`wm doctor` muestra la versión de Windows, el modo de DPI y los monitores conectados (resolución, frecuencia, escala y área de trabajo).

Las pruebas incluyen 10.000 secuencias aleatorias de teclado que verifican que la tecla Hyper nunca deje teclas trabadas.

## Estructura

```
app/                 # binario único `wm`
crates/wm-model/     # núcleo puro: geometría, layouts, estado (sin unsafe)
crates/wm-platform/  # capa Win32
settings/            # ventana de ajustes en Slint (wm-settings.exe, spike 8)
tools/wm-bench/      # mediciones de los spikes (aparte, para no cargarle D3D a wm.exe)
assets/fonts/geist/  # la tipografía (OFL), incluida en wm.exe y wm-settings.exe
```

## Licencia

[GPL-3.0-or-later](LICENSE).

La tipografía [Geist](https://github.com/vercel/geist-font) va incluida en los ajustes y en los overlays bajo su propia licencia, la [SIL Open Font License 1.1](assets/fonts/geist/OFL.txt).
