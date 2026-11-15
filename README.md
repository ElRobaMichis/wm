# wm (nombre provisorio)

Gestor de ventanas para Windows, escrito en Rust. Pensado para que cualquiera lo use sin leer documentación: pocos atajos lógicos, mouse como opción de primera clase, consumo mínimo y animaciones fluidas a la frecuencia real del monitor.

> **Estado:** spikes 1 a 7 hechos (el 7 espera las pruebas manuales de pantallas): `Caps` como tecla Hyper e ir a una app por su inicial, esconder y mostrar ventanas sin parpadeo, un guardián que nunca deja ventanas perdidas, un modelo de hilos que no se traba con apps colgadas ni con la CPU al 100 %, overlays animados por DWM a la frecuencia del monitor, y animación de ventanas con un "fantasma" sin destellos. Todavía no mueve ventanas ni tiene escritorios propios (ver [architecture.md](architecture.md) §13).

## Probar el prototipo

```bash
cargo build --release
target/release/wm guardian
```

`wm guardian` lanza `wm core` y lo vigila: si se cae, muestra las ventanas que haya escondido y lo vuelve a lanzar (como mucho 3 veces por minuto). También se puede correr `wm core` solo.

Con `wm core` corriendo:

| Teclas | Qué hace |
|---|---|
| `Caps` + letra | Va a la app con esa inicial (por ejemplo `Caps` + `B` = Brave). Repetir = siguiente ventana de esa app |
| `Caps` (toque corto) | Bloq Mayús normal |
| `Caps` + `Esc` o `Ctrl+C` en la consola | Salir; muestra la latencia del hook |

`wm apps` muestra a qué app apunta cada letra **sin mover el foco**.

Si alguna vez una ventana queda escondida, `wm restore` la muestra (y `wm hidden` lista las que tengan la marca de `wm`).

Sin configuración, cada letra va a la app abierta más reciente cuyo nombre de ejecutable empieza con esa letra. Para fijar letras, crear `%APPDATA%\wm\apps.txt`:

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
tools/wm-bench/      # mediciones de los spikes (aparte, para no cargarle D3D a wm.exe)
```

## Licencia

[GPL-3.0-or-later](LICENSE).
