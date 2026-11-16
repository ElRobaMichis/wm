# wm (nombre provisorio)

Gestor de ventanas para Windows, escrito en Rust. Pensado para que cualquiera lo use sin leer documentación: pocos atajos lógicos, mouse como opción de primera clase, consumo mínimo y animaciones fluidas a la frecuencia real del monitor.

> **Estado:** en desarrollo, hitos 0 a 3 de la primera versión hechos ([plan-core.md](plan-core.md)): `Caps` + letra va a una app; `Caps` + Alt acomoda la ventana al frente (mitades, esquinas, otro monitor, maximizar, centrar, agrandar y achicar, deshacer) con animación; mantener `Caps` muestra el mapa de teclado. Arranca con `wmw` (sin consola, con ícono en la bandeja) y se puede iniciar con Windows. Faltan los escritorios 1–4 (hito 4) y el mouse (hito 5).

## Probar

```bash
cargo build --release
target/release/wmw
```

`wmw.exe` es `wm` sin ventana de consola: arranca el guardián, que lanza el core y lo vuelve a lanzar si se cae (mostrando antes todas las ventanas que haya escondido). Aparece un ícono en la bandeja:

- **clic:** abre los ajustes;
- **clic derecho:** Ajustes, Pausar atajos, Deshacer el último movimiento, Iniciar con Windows y Salir.

`wm.exe` es el mismo programa con consola (para ver el log en vivo y para la línea de comandos): `wm` sin argumentos hace lo mismo que `wmw`; `wm core` corre solo el core.

| Teclas | Qué hace |
|---|---|
| `Caps` + letra | Va a la app con esa inicial (por ejemplo `Caps` + `B` = Brave). Repetir = siguiente ventana de esa app |
| `Caps` + Alt + W/A/S/D | Esa mitad; repetir = al otro monitor; dos juntas = esquina |
| `Caps` + Alt + F / C / Z / G / V | Maximizar / centrar / deshacer / agrandar / achicar |
| Mantener `Caps` | El mapa de teclado (con Alt, la capa de ventana) |
| `Caps` (toque corto) | Bloq Mayús normal |

Línea de comandos (con `wm` corriendo): `wm quit`, `wm pause`, `wm resume`, `wm undo`, `wm ajustes`; `wm autostart on|off` (iniciar con Windows); `wm apps` muestra a qué app apunta cada letra; `wm restore` muestra cualquier ventana que haya quedado escondida. El log está en `%LOCALAPPDATA%\wm\wm.log`.

Sin configuración, cada letra va a la app abierta más reciente cuyo nombre de ejecutable empieza con esa letra. Para fijar letras, usar los ajustes o editar `%APPDATA%\wm\apps.txt` (se recarga solo al guardar):

```text
# letra = ejecutable (sin .exe) | ruta para abrirla si está cerrada (opcional)
b = brave
t = windowsterminal
n = notion | C:\Users\yo\AppData\Local\Programs\Notion\Notion.exe
```

## Documentos

- [findings.md](findings.md): investigación de usuarios y mercado.
- [keyboard-design.md](keyboard-design.md): modelo de interacción y mapa de atajos ("Apps primero").
- [architecture.md](architecture.md): arquitectura técnica y plan de spikes.
- [design.md](design.md): lenguaje de diseño (principios, tokens y componentes).

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
