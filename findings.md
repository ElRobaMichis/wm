# Findings: gestores de ventanas (macOS, Windows y Linux)

Investigación de usuarios y mercado para diseñar nuestro propio gestor de ventanas.

- **Fecha:** noviembre 2026
- **Fuentes:** unos 40 hilos de Reddit (r/MacOS, r/macapps, r/MacOSApps, r/mac, r/applesucks, r/Windows11, r/windows, r/desktops, r/linuxquestions, r/rust, r/programming, r/gnome, r/xfce, r/Kubuntu, r/archlinux, r/i3wm, r/neovim, r/UsabilityPorn, r/MacStack, r/techsupport) y los README de GitHub de **yabai**, **AeroSpace**, **komorebi** y **niri**. En la segunda ronda se sumaron comentarios de YouTube (videos sobre AeroSpace/yabai y sobre tiling en Linux) y la transcripción de un video que instala GlazeWM desde cero (ver §16).
- **Aclaración metodológica:** la "popularidad" de cada herramienta se basa en **cuántas veces aparece** en los hilos y en las estrellas de GitHub, no en datos de descargas. Las citas se dejan en inglés, tal como las escribieron sus autores.
- Las notas marcadas con **(nota propia)** son conocimiento técnico agregado por nosotros, no salieron de las fuentes.

---

## Índice

1. [Resumen ejecutivo](#1-resumen-ejecutivo)
2. [Cómo funciona cada sistema hoy](#2-cómo-funciona-cada-sistema-hoy)
3. [Mapa de herramientas](#3-mapa-de-herramientas)
4. [Los tres grandes: yabai, AeroSpace y komorebi](#4-los-tres-grandes-yabai-aerospace-y-komorebi)
5. [Lo que la gente ama](#5-lo-que-la-gente-ama)
6. [Lo que la gente odia](#6-lo-que-la-gente-odia)
7. [Lo que la gente quiere y nadie hace](#7-lo-que-la-gente-quiere-y-nadie-hace)
8. [Funciones nativas que casi nadie conoce](#8-funciones-nativas-que-casi-nadie-conoce)
9. [Perfiles de usuario](#9-perfiles-de-usuario)
10. [Problemas técnicos conocidos](#10-problemas-técnicos-conocidos)
11. [Por qué la gente desinstala en los primeros 15 minutos](#11-por-qué-la-gente-desinstala-en-los-primeros-15-minutos)
12. [Mercado, precio, licencia y lanzamiento](#12-mercado-precio-licencia-y-lanzamiento)
13. [Principios de diseño](#13-principios-de-diseño)
14. [La oportunidad y preguntas abiertas](#14-la-oportunidad-y-preguntas-abiertas)
15. [Anexo: citas destacadas](#15-anexo-citas-destacadas)
16. [Segunda ronda: YouTube, niri y el caso GlazeWM](#16-segunda-ronda-youtube-niri-y-el-caso-glazewm)
17. [Tercera ronda: tutoriales de yabai, AeroSpace y Raycast](#17-tercera-ronda-tutoriales-de-yabai-aerospace-y-raycast)

---

## 1. Resumen ejecutivo

- **Ninguna plataforma tiene el problema resuelto, y cada una envidia algo de las otras.**
  - Mac y Linux envidian el **Snap Layouts de Windows 11**.
  - Windows envidia los **Spaces de macOS**: escritorios independientes por monitor y pantalla completa en su propio espacio.
  - Ambos envidian el **tiling de Linux** (Hyprland, i3, niri).
- **El uso masivo está en herramientas simples:** Rectangle, Magnet y Loop en Mac; el snap nativo y FancyZones en Windows. Los tilers (AeroSpace, yabai, komorebi, GlazeWM) son un nicho de usuarios avanzados, muy ruidoso en Reddit.
- **Los tres tilers grandes apuntan al mismo público:** teclado, archivos de configuración y CLI. AeroSpace declara que **nunca** tendrá una interfaz gráfica de configuración. Eso deja libre el espacio de los usuarios que usan mouse o trackpad y no quieren configurar.
- **La queja de fondo no es el "snap", es el estado.** *"Window managers exist because people want state, not just snapping."* La gente quiere que el sistema recuerde layouts, monitores, reposo y grupos de apps, y que lo aplique solo.
- **Los dos grandes huecos:**
  1. Un modelo **híbrido de zonas fijas, pilas de ventanas y tiling automático**.
  2. **Persistencia automática y confiable**: varios monitores, dock, reposo, reglas por app.
- **Problemas técnicos que nadie resolvió en macOS:** pestañas nativas, bloqueos de la Accessibility API y apps con tamaños mínimos.
- **Mercado:** en Mac se puede cobrar (pago único de 10–20 USD). En Windows casi todo es gratis (PowerToys es de Microsoft), así que hay que entrar por un nicho y con licencia gratuita o permisiva.
- **La oportunidad en una frase:** un gestor visual que se use con mouse, combine zonas fijas y tiling con pilas, recuerde todo automáticamente y no se rompa.

---

## 2. Cómo funciona cada sistema hoy

### 2.1 macOS

**El modelo**
- **Centrado en aplicaciones, no en ventanas.**
  - `Cmd+Tab` cambia de app, no de ventana.
  - El Dock muestra un ícono por app.
  - Hacer clic en el Dock trae **todas** las ventanas de esa app.
- **Organiza por Spaces** (escritorios virtuales, desde 2006) y por gestos de trackpad. Según la explicación más valorada de r/MacOS, el sistema está pensado para administrar espacios de trabajo, no ventanas sueltas.
- **Ventanas superpuestas** "como papeles sobre el escritorio": es la filosofía histórica.
- **Pantalla completa** (botón verde) manda la ventana a un Space propio. Para maximizar sin pasar a pantalla completa hay que usar `Option` + clic en el botón verde, doble clic en la barra de título (si está configurado en "Fill") o arrastrar la ventana a la barra de menú.
- **Tiling nativo (Sequoia, macOS 15+):**
  - arrastrar la ventana a los bordes,
  - menú del botón verde,
  - atajos `Fn+Ctrl+flechas`.
  - Funciona, pero es lento (tiene un retraso al llegar al borde) y casi nadie conoce los atajos.
- **Stage Manager:** algunos lo aman y otros lo odian.

**Problemas del sistema**
- **`Cmd+``** cicla las ventanas de la app actual, pero:
  - solo en el Space actual,
  - no anda en teclados que no son de EE. UU.,
  - cada app lo implementa distinto (Adobe no lo respeta),
  - no llega a una ventana en pantalla completa de la misma app.
- **Varios monitores:**
  - las ventanas nuevas se abren en el monitor equivocado,
  - el Dock salta de un monitor a otro y no se puede fijar,
  - el foco no sigue al clic: hay que hacer dos clics,
  - las teclas pueden ir a la app equivocada,
  - al despertar un monitor DisplayPort/USB-C, las ventanas se mudan de pantalla.
- **"Automatically rearrange Spaces"** viene activado por defecto y desordena todo.
- **No hay atajo de teclado para mover una ventana a otro Space o monitor.**
- **Mission Control** muestra miniaturas chicas y en posiciones que cambian, así que la memoria espacial no sirve.
- **Apps que fuerzan pantalla completa** (Zoom, juegos, videos de YouTube) no tienen forma de quedarse en el escritorio actual.
- **No existe focus-follows-mouse.**
- **Los gestores de terceros solo pueden usar la Accessibility API:** Apple no ofrece una API de gestión de ventanas.

### 2.2 Windows 11

**El modelo**
- Centrado en ventanas, con barra de tareas y `Alt+Tab` por ventana.
- **El snap es el mejor de las tres plataformas:**
  - Snap Layouts: pasar el mouse sobre maximizar o `Win+Z`,
  - `Win+flechas`,
  - Snap Assist: después de acomodar una ventana ofrece las demás para llenar el resto,
  - grupos de snap.
  - El menú de layouts se diseñó **para que la función se descubra fácil** (lo dijo una ingeniera de Microsoft en Reddit).
- **El menú se adapta al monitor:** muestra entre 4 y 6 layouts según tamaño y escala, y layouts arriba/abajo en monitores verticales.

**Problemas del sistema**
- **Cuesta predecir qué va a hacer** (cita en el [anexo §15](#15-anexo-citas-destacadas)):
  - si aceptás la sugerencia de Snap Assist, las ventanas quedan vinculadas y se redimensionan juntas; si la descartás, no;
  - apretar `Win+→` dos veces manda la ventana a otro monitor;
  - el resultado depende de cosas que hiciste antes y que no se ven.
- **Escritorios virtuales débiles:**
  - no son independientes por monitor: cambian todos a la vez,
  - mover una ventana a otro escritorio es incómodo,
  - no hay "ir al escritorio N",
  - **no hay API pública para manejarlos.**
- **La barra de tareas** agrupa por app; al principio Windows 11 no dejaba desagruparla.
- **Las ventanas cambian de tamaño** al volver del reposo o al despertar el monitor, y la barra pierde el registro de qué ventana está en cada pantalla.
- **Las ventanas nuevas tapan a otras** aunque haya espacio libre, y no se abren en el monitor del puntero (KDE sí lo hace).
- **No se pueden desactivar las animaciones de minimizar y maximizar** que usan los tilers.
- **No hay espacios entre ventanas ni layouts espejados** en el snap nativo.

### 2.3 Linux (como referencia)

- **Entornos de escritorio:**
  - KDE tiene editor de tiling (`Meta+T`) y `Shift` + arrastrar,
  - GNOME tiene la extensión Tiling Shell (una copia de FancyZones con Snap Assist),
  - KZones es FancyZones para KDE.
- **Gestores de ventanas:** i3, Sway, Hyprland, bspwm, dwm y **niri** (desplazamiento horizontal).
- **Tipos de tiling:**
  - *manual*: i3, donde el usuario decide hacia dónde se divide;
  - *dinámico/automático*: bspwm, Hyprland, komorebi;
  - *desplazamiento horizontal*: niri, Paneru y scroll. Las ventanas forman una tira infinita y abrir una nueva **no achica** las demás.
- **El debate en Linux:** el tiling **no es esencial**. Muchos usuarios avanzados prefieren ventanas flotantes con buen snapping y atajos. El tiling gana con teclado, terminal y pantallas chicas; con varios monitores y mouse, las ventanas flotantes a veces son más prácticas.

---

## 3. Mapa de herramientas

### 3.1 macOS: snapping y zonas (uso masivo)

| Herramienta | Precio | Fortaleza | Debilidad o comentario |
|---|---|---|---|
| **Rectangle** | Gratis, open source | La respuesta por defecto; muchísimos atajos, rápido | No aplica layouts automáticamente |
| **Rectangle Pro** | ~10 USD, pago único | Layouts de apps, CLI, tamaños propios, "throw" con gestos, iCloud | *"Best and most reliable"* |
| **Magnet** | ~4–8 USD (App Store) | Simple, popular entre quienes vienen de Windows | No tiene perfiles de layout |
| **Loop** | Gratis, open source | Menú radial con `Fn`, Cycles, se ve nativo | Crece mucho en recomendaciones |
| **Moom** | Pago único | Layouts guardados, AppleScript, perfiles por cantidad de monitores | No aplica layouts solo al cambiar de monitores |
| **Swish** | Pago único | Gestos de trackpad sobre la barra de título | Sirve con trackpad o Magic Mouse; con mouse, no |
| **BetterSnapTool / BetterTouchTool** | Pago | Muy configurable | Complejo |
| **Raycast** | Gratis (lanzador) | Manejo de ventanas incluido | Algunos lo encuentran lento |
| **Lasso** | Pago | Grilla (sucesor de Divvy) | — |
| **MacsyZones** | Gratis (Pro opcional) | FancyZones para Mac, Snap Resize, Shake to Snap | Le costó funcionar bien con varios monitores |
| **BentoBox** | ~9 USD | Al estilo FancyZones, clic derecho o `Shift` | Piden zonas superpuestas |
| **Wins** | Pago | El más "como Windows": vista previa en el Dock, Aero Shake | Propietario |
| **Mosaic, 1Piece, Tiles, Better Window Manager, MacTiler, Clippper, WindowKeys** | Variado | Nichos | — |

### 3.2 macOS: tiling (nicho)

| Herramienta | Estado | Notas |
|---|---|---|
| **AeroSpace** | Beta pública, 23.8k ⭐, el que más crece | Al estilo i3, workspaces propios, no pide desactivar SIP. Sin tiling automático (planeado para después de la 1.0) |
| **yabai** (+ skhd) | Maduro, 29.8k ⭐ | BSP con Spaces nativos; para las funciones avanzadas hay que desactivar SIP |
| **Amethyst** | En baja | Fácil de usar, pero se rompe con las actualizaciones |
| **Rift** | Emergente | Workspaces virtuales y APIs privadas; las mejores animaciones |
| **Glide** (Rust) | Emergente | Sway/i3 integrado a los Spaces nativos, animaciones, layouts por tamaño de pantalla, tres columnas. *"First tiling WM that immediately feels intuitive"* |
| **Paneru** | Emergente | Desplazamiento horizontal al estilo niri |
| **HyprMac** | Emergente | Al estilo Hyprland, Caps Lock como tecla Hyper, demo web interactiva |
| **WinMux** | Beta | AeroSpace con barra lateral, pestañas, interfaz de configuración y modo no administrado; un solo monitor |
| **komorebi para Mac, OmniWM, KiwiDesk, Ordinary Space, Tessel, Wimi** | Nuevos | Ver §7 |
| **Hammerspoon** | Framework en Lua | Programable al máximo; hoy se le pide al LLM que escriba la configuración |

### 3.3 macOS: selectores, Dock y persistencia

| Categoría | Herramientas | Notas |
|---|---|---|
| Selector por ventana | **AltTab** (gratis, el más recomendado), Witch, rcmd, Contexts, BetterCmdTab, **fswitcher** (por monitor), Tangrid | A AltTab se le critica que se llena de ventanas, que "se ve como Windows" y que controla el monitor equivocado |
| Vista previa en el Dock | **DockDoor**, Wins, Dock Exposé | A DockDoor se lo ve "un poco buggy, es nuevo" |
| Reemplazos del Dock | Sidebar, Taskbar, Tasky, WindowShelf | Nicho |
| Restaurar layouts | **Stay** (gratis ahora, sin mantenimiento), **Spencer** (incluye Spaces), Display Maid (abandonado), SnapState, Snapper | Muy demandado |
| Spaces | SpaceJump, CurrentKey Stats, WhichSpace | Para nombrarlos y saltar entre ellos |
| Barras | SketchyBar + JankyBorders, Aegis, AeroSpaceBar | Para personalizar el escritorio ("ricing") |
| Directorios | macoswm.com, planilla de r/macapps | Conviene figurar ahí |

### 3.4 Windows

| Herramienta | Tipo | Notas |
|---|---|---|
| **Snap nativo de Windows 11** | Snap | La mayoría no instala nada más |
| **FancyZones (PowerToys)** | Zonas | Gratis y de Microsoft. Lo más querido, sobre todo en ultrawide. Clic derecho o `Shift` + arrastrar, espacios entre ventanas, `Win+PgUp/PgDn` dentro de una zona |
| **PowerToys Workspaces** | Persistencia | Guarda y abre grupos de apps |
| **DisplayFusion** | Varios monitores | Mantiene las posiciones al cambiar de resolución |
| **komorebi** | Tiling | 15.4k ⭐. El más completo; uso comercial pago |
| **GlazeWM** | Tiling | GPL, `.exe` portable, sin permisos de administrador, ideal en PCs de trabajo. Barra Zebar. Lo consideran más rápido y estable que komorebi |
| Whim, Jwno, JigsawWM (Python), FancyWM, workspacer, nog, bug.n, b3, dwm-win32 | Tiling | Minoritarios o inestables |
| AutoHotkey + VirtualDesktopAccessor, StrokesPlus | Automatización | Soluciones artesanales |
| yasb, Zebar, Flow Launcher, Windhawk, ButteryTaskbar | Complementos | Personalización |
| StageManagerForWindows, stage-bar, MyDockFinder | Al estilo Stage Manager | Todos flojos: hay demanda sin cubrir |

---

## 4. Los tres grandes: yabai, AeroSpace y komorebi

### 4.1 Datos (noviembre 2026)

| | **yabai** (macOS) | **AeroSpace** (macOS) | **komorebi** (Windows) |
|---|---|---|---|
| Estrellas | 29.8k | 23.8k (eran ~12.5k hace 2 años) | 15.4k |
| Forks | 817 | 807 | 380 |
| Contribuidores | **5** | 60 | 57 |
| Lenguaje | C + Objective-C | Swift | Rust |
| Licencia | MIT | MIT | Komorebi 2.0.0 (derivada de PolyForm Strict; no es open source según la OSI) |
| Versiones | 106 (v7.1.25) | 64 tags, beta | 43 (v0.1.41) |
| Actividad | Commit hace 3 días | Commit hace 2 días | Commit hace 2 semanas |
| Financiamiento | Patreon y GitHub Sponsors | Sponsors (tiempo libre del autor) | Sponsors, Ko-fi, YouTube y **licencia comercial por suscripción** |

### 4.2 Filosofías

**yabai: integración profunda con macOS**
- **Funciona como extensión del gestor nativo.** Usa los Spaces y organiza las ventanas con BSP (divide el espacio en dos una y otra vez).
- **Para sus funciones avanzadas** hay que desactivar SIP parcialmente: inyecta código en el Dock para controlar el WindowServer.
- **Funciones únicas:** focus-follows-mouse, quitar las animaciones de los Spaces, más de 16 Spaces, animaciones de ventanas (piden permiso de Grabación de Pantalla).
- **Exige cambiar ajustes de macOS:**
  - "Displays have separate Spaces" activado,
  - "Automatically rearrange Spaces" desactivado,
  - "Show Items On Desktop" activado,
  - "Click wallpaper to reveal Desktop" en "Only in Stage Manager".
- **Admite que las pestañas nativas no funcionan** y recomienda dejar esas ventanas flotando.
- **Riesgo:** depende prácticamente de un solo mantenedor.

**AeroSpace: independencia y robustez**
- **Al estilo i3**, con estructura de árbol. **Emula sus propios workspaces** e ignora los Spaces de macOS; por eso el cambio es instantáneo.
- **Usa una sola API privada** (`_AXUIElementGetWindow`); todo lo demás es la Accessibility API pública. Nunca pedirá desactivar SIP. El objetivo es aguantar las actualizaciones de macOS.
- **Configuración en TOML**, CLI primero, páginas de manual y autocompletado.
- **Rechaza explícitamente:**
  - una interfaz gráfica de configuración,
  - bordes, transparencias y animaciones,
  - la personalización estética,
  - integrarse con las funciones nativas de macOS.
- **No está notarizado.** Homebrew le quita la marca de cuarentena.
- **Los reportes se abren primero como Discussion** en GitHub.
- **Pendiente para la 1.0:**
  - un hilo por app para evitar los bloqueos de la Accessibility API (#131),
  - reescribir el núcleo como un árbol inmutable (#1215),
  - corregir las ventanas que saltan al workspace activo (#1216),
  - soportar pestañas nativas (#68),
  - combinadores de comandos (#278),
  - investigar `CGEvent.tapCreate` para atajos globales y distinguir los modificadores izquierdo y derecho.
- **Después de la 1.0:** ventanas fijas en todos los workspaces (#2) y **tiling dinámico** (#260).
- **Truco incluido en el README:** `defaults write -g NSWindowShouldDragOnGesture -bool true` permite mover una ventana con `Ctrl+Cmd` + arrastrar desde cualquier parte.

**komorebi: extensión de DWM con ecosistema**
- **Extiende DWM** y modifica el sistema lo mínimo posible.
- **Se controla por CLI** (`komorebic`); los atajos van con whkd o AutoHotkey.
- **Componentes:** komorebi-bar, komorebi-gui, komorebi-shortcuts (egui), temas, layouts, **desplazamiento horizontal** (con paneo por trackpad), animaciones de workspace y bordes.
- **Pensado para que otros construyan encima:**
  - `komorebic state` devuelve el estado completo como JSON,
  - suscripción a eventos por named pipes, Unix sockets o TCP,
  - cliente en Rust (`komorebi-client`),
  - esquemas JSON de mensajes y notificaciones.
- **Esconde ventanas** con el *cloaking* oculto de los escritorios virtuales de Windows.
- **Recuperación:** guarda los identificadores de ventana en `komorebi.hwnd.json` y `komorebic restore-windows` recupera las escondidas aunque el proceso principal haya muerto.
- **Regla de oro:** *"Breaking changes to user-facing interfaces are unacceptable"*. Ningún usuario debería ver su configuración rota tras actualizar.
- **Licencia:**
  - el uso comercial requiere una licencia individual por suscripción,
  - **detecta si la PC está administrada por una empresa (MDM)** y muestra un aviso,
  - hay excepción manual para estudiantes.
  - Efecto en Reddit: *"I can't use komorebi at work… my boss won't go for that"* → la persona se fue a GlazeWM.
  - El autor sostiene que con la licencia nueva **aumentaron** las contribuciones.

### 4.3 Qué tomar de cada uno

| De | Tomar | Evitar |
|---|---|---|
| yabai | Funciones únicas (focus-follows-mouse, más Spaces) como opcionales | Depender de desactivar SIP; un solo mantenedor |
| AeroSpace | Solo APIs públicas, workspaces propios, velocidad, robustez | Rechazar la interfaz gráfica; no notarizar; ignorar los Spaces y gestos nativos |
| komorebi | Eventos y estado en JSON, recuperación de ventanas, no romper la configuración, ecosistema | Licencia que frena el uso en el trabajo; detección de MDM |

---

## 5. Lo que la gente ama

### 5.1 Velocidad
- Cambiar de workspace sin animación (AeroSpace): *"I can't live with the fast workspaces switching now"*, *"priceless"*.
- Rectangle "más rápido que el nativo". Hammerspoon con animación en 0: *"feel really snappy"*.
- Cambio instantáneo de app (`Cmd` derecho + inicial, fswitcher y rcmd).

### 5.2 No tener que pensar
- Tiling: abrís la ventana y aparece en su lugar, sin arrastrar ni redimensionar. *"Window managers… you don't have to fiddle with windows."*
- Workspaces con una app asignada a cada uno: el cambio se vuelve memoria muscular.

### 5.3 Memoria muscular y "no hay vuelta atrás"
- *"Any computer without it now feels broken somehow. Just like browsing the web without an adblocker."*
- *"Once you've learned a tiling manager… nothing else compares."*

### 5.4 Instalar y que funcione
- AeroSpace por Homebrew, sin desactivar SIP.
- GlazeWM: un `.exe` que no requiere permisos de administrador. Clave en PCs de trabajo.
- Rectangle: funciona sin configurar nada.

### 5.5 Confiabilidad a largo plazo
- Stay: *"never failed… two offices and home… 4 monitor environments"*.
- Moom: *"10 years, works flawlessly"*.

### 5.6 Restaurar layouts con una acción
- Moom (layouts y perfiles por cantidad de monitores), Rectangle Pro (App Layouts + CLI + Alfred), Spencer (*"just click Save… so simple"*, también restaura Spaces).
- Tessel: los layouts sobreviven al reinicio y las ventanas vuelven al reconectar el monitor.

### 5.7 Zonas propias para monitores grandes
- FancyZones en ultrawide: centro 21:9 con laterales, o 16:9 con laterales 8:9. *"I'll never go back to using two screens."*
- Rectangle Pro con grilla 4×2 y teclado numérico.

### 5.8 Interacción natural
- **Swish:** gestos sobre la barra de título; *"the fastest, most natural interface I've ever used"*.
- **Loop:** menú radial con `Fn` y Cycles (repetir el atajo alterna ½ → ⅓ → ⅔).
- **Swift Shift / `Ctrl+Cmd` + arrastrar:** mover y redimensionar desde cualquier parte de la ventana.
- **Arrastrar con tres dedos** (configuración de accesibilidad de macOS).

### 5.9 Redimensionado vinculado
- Tessel y MacsyZones (Snap Resize): mover un divisor redimensiona las dos ventanas vecinas.
- En Windows, los grupos de snap lo hacen, pero solo si aceptaste Snap Assist.

### 5.10 Ideas nuevas
- **Pestañas que mezclan apps** (Tessel): Mail, Calendar y Notes en una misma celda, y `Cmd+Tab` respeta el grupo.
- **Ventana fijada en vivo** encima de las demás (Tessel).
- **`Cmd+Tab` por monitor** (fswitcher).
- **Apps sin ventanas en una fila aparte**, y que al elegirlas se abra una ventana (fswitcher).
- **Intercambiar el contenido completo** entre dos monitores (MacTiler).

### 5.11 Desarrolladores que responden
- Spencer arregló un bug en 24 horas. El desarrollador de komorebi responde en todos los hilos. Tiling Shell recibió 4 issues en un solo hilo y los atendió. Tessel publicó varias betas en una semana a partir de comentarios.

### 5.12 Precio justo
- Gratis o pago único de 10–20 USD. *"I just want to pay the developer to keep working on it."*

### 5.13 Estética nativa
- Loop: *"best looking app of any window manager I've used"*, con animaciones parecidas a las de Apple.
- Las animaciones ayudan a seguir con la vista adónde fue cada ventana (Rift, KiwiDesk, Hyprland).

---

## 6. Lo que la gente odia

### 6.1 Que se rompa con las actualizaciones
- Amethyst: *"stopped working after every OS update"*. Display Maid dejó de funcionar. Stay quedó sin mantenimiento.
- yabai tiene que adaptarse a cada versión de macOS (los commits recientes son "Tahoe fixes", "Golden Gate").

### 6.2 Proyectos abandonados
- Divvy, Display Maid, Stay, StageManagerForWindows, Spectacle, Cinch.
- La fecha de la última actualización es un factor de compra: *"I'm cautious of Stay due to infrequent updates and lack of communication from developer."*

### 6.3 Configurar con archivos
- *"I am not good with configs"* (de alguien que viene de Linux y usaba configuraciones ajenas).
- La gente le pide al LLM que le escriba la configuración de AeroSpace, SketchyBar o Hammerspoon.
- Curva de aprendizaje: alguien **pausaba AeroSpace durante las reuniones** porque no recordaba los atajos.

### 6.4 Comportamiento errático
- Ventanas que **rebotan** entre posiciones (Tessel, por calcular mal los tamaños mínimos).
- Ventanas que **saltan solas** al workspace activo (AeroSpace #1216).
- Ventanas que **quedan escondidas** detrás de otras (AeroSpace).
- Bordes con marcas visuales y que siguen visibles durante la animación de minimizar (komorebi).
- Ventanas que se superponen después de corregir su tamaño.

### 6.5 Varios monitores
- *"Aerospace with multiple monitors… quite buggy sometimes."*
- FancyWM se cuelga cuando cambia la cantidad de monitores (la pantalla de bloqueo usa uno solo).
- MacsyZones no funcionaba con tres monitores (lo corrigió la v1.5).
- WinMux directamente solo soporta un monitor.

### 6.6 Que todo cambie de tamaño al abrir una ventana
- *"I hate that when I open a new window, the sizes of my existing windows are liable to change."* Es la razón del auge de niri y los gestores con desplazamiento horizontal.

### 6.7 Perder libertad
- *"I want the chaos + overlapping tiling zones. I control when and IF to tile."*
- *"I don't want a window manager that tries to auto-arrange all windows. That's too dynamic for me."* (usuario de Rectangle Pro)
- Pedido de accesibilidad: dejar espacios entre ventanas y bordes para ver dónde termina cada una y poder agarrarla.

### 6.8 Apps que se resisten
- Adobe, paletas flotantes, la barra lateral flotante de Fusion, herramientas EDA, X11, diálogos y modales.
- Apps con tamaño mínimo (Xcode, Slack) en divisiones angostas.
- Pestañas nativas (Finder, Ghostty, Terminal, TablePlus, Alacritty).
- Juegos en modo ventana: GlazeWM los tapa en vez de acomodar las demás ventanas alrededor.

### 6.9 Latencia
- *"After using GlazeWM I feel latency"* con komorebi (atribuido posiblemente a whkd).
- Raycast: *"extremely slow"* para manejar ventanas.
- El retraso del tiling nativo de macOS al llegar al borde. El menú del botón maximizar de Windows "tarda mucho en aparecer".

### 6.10 Tener que unir varias apps
- SketchyBar + AeroSpace + skhd + Karabiner. WinMux nació para unificarlas.
- yabai necesita skhd; komorebi necesita whkd o AutoHotkey.
- A la vez: **funciones que nadie pidió** también molestan (Show Bar, ver §6.13). El equilibrio es una sola app **enfocada** en las ventanas.

### 6.11 Requisitos que dan miedo
- Desactivar SIP (yabai): *"I'm concerned of disabling some kernel protection features."*
- Apps sin notarizar (AeroSpace por fuera de Homebrew).
- El permiso de Grabación de Pantalla genera desconfianza y, en Tessel, el aviso seguía apareciendo después de denegarlo.
- El aviso *"find devices on local network"* (MacTiler, por la validación de licencia y las analíticas).

### 6.12 Precio y licencias
- Suscripción semanal: *"Sorry, no. Just no."*
- Raycast Pro (~100 €/año) descartado.
- Licencia que no permite uso en el trabajo (komorebi).

### 6.13 Funciones que nadie pidió
- Show Bar traía capturas, grabación de pantalla e historial del portapapeles: *"Thanks but no thanks"*, desinstalado.

### 6.14 Selectores recargados o que no se ven nativos
- AltTab muestra todas las ventanas y se llena. Además controla todos los monitores a la vez.
- *"AltTab looks really like Windows… that's a meh for me."*

### 6.15 Marketing deshonesto
- Imagen generada con IA y texto ilegible, decir "macOS no evolucionó" sin haber probado alternativas, no explicar qué hace diferente: *"clone of a clone of a clone that someone vibe-coded"*, *"guerrilla marketing"*.

---

## 7. Lo que la gente quiere y nadie hace

Ordenado por grupo. Los marcados ⭐ son los que vemos como mayor oportunidad.

### 7.1 Layout y tiling

1. ⭐ **Modelo híbrido: zonas fijas más tiling automático**, pudiendo cambiar entre los dos.
   - Pedido de un usuario de ultrawide de 49":
     - una zona central fija de 2560×1080 que no cambie,
     - apilar ventanas en una zona y alternar con un atajo,
     - *"cycle between zones and automatic whenever you want"*,
     - atajos por zona (`Alt+1`, `Alt+2`).
   - En XFCE: *"existing solutions are dynamic tiling only, not snapping a window to a predefined area"*.
2. ⭐ **Áreas de trabajo con pilas de ventanas:** un layout como [30 | 50 | 20] por escritorio, cada área con su pila, y la ventana nueva entra al área activa. *"I don't really want auto tiling, I want fixed workareas."* Solo lo hace Ordinary Space, que es muy nuevo.
3. **Ventanas con tamaño fijo:** el reproductor de música con ancho mínimo que no cambia al abrir o cerrar otras; el tiling se acomoda alrededor de una ventana fija (por ejemplo, un juego centrado).
4. ⭐ **Reglas por app:**
   - tamaño mínimo y máximo, lado preferido de la pantalla,
   - flotar o ignorar,
   - posición al abrir.
   - Ejemplo: "navegador entre 50 y 60 % de ancho y a la izquierda; las demás ventanas se acomodan dentro de esos límites".
5. **Snap Assist en macOS:** *"I haven't found an app that lets me snap and then brings up a view of other open apps to put in the split."*
6. **Redimensionado vinculado en los dos ejes** (Tiling Shell solo lo hacía en horizontal).
7. **Activar el gestor solo en algunos monitores** (pedido a JigsawWM).
8. **Ventanas fijas que se muestran en todos los workspaces** (AeroSpace #2).
9. **Pestañas que mezclan apps** (solo Tessel, beta).

### 7.2 Persistencia y estado

10. ⭐ **Aplicar el layout automáticamente:**
    - al abrir una app (Display Maid lo hacía y está abandonado; Rectangle obliga a activarlo a mano),
    - al conectar o desconectar monitores (Moom tampoco lo hace solo),
    - al volver del reposo o al despertar el monitor,
    - y que las ventanas vuelvan a su monitor al reconectarlo.
11. **Grupos de apps** que se abren juntos con tamaño y posición (*"remember my app groups… so I can quickly fire up my work"*). Que incluyan los Spaces (Spencer lo hace en parte).
12. **Layouts distintos según el tamaño de pantalla**, con dock y sin dock (Glide lo hace y es lo más elogiado).
13. **Abrir las ventanas nuevas en el monitor del puntero** y **no tapar otra ventana si hay espacio libre**.

### 7.3 Navegación y cambio de ventana

14. ⭐ **Selector por ventana y por monitor**, con posiciones estables para usar la memoria espacial, nombres editables y apps sin ventanas aparte (hoy solo fswitcher, y es pago).
15. **Al estilo Stage Manager:** ventana principal en el centro y miniaturas en vivo de las demás al costado. Motivado por la **ergonomía**: *"if I spend too long looking toward one corner, my neck starts to hurt"*. En Windows no hay nada bueno.
16. **Ponerles nombre e ícono a los Spaces o workspaces** y moverse entre ellos con el teclado.
17. **Mover una ventana a otro Space o monitor desde el teclado** (macOS no lo hace de forma nativa).
18. **Desplazamiento horizontal para ultrawide:** en Windows no existe (komorebi recién lo agrega) y en Mac solo está Paneru.
19. **Focus-follows-mouse sin desactivar SIP** (hoy solo yabai, con SIP desactivado).
20. **`Cmd+Tab` que llegue a ventanas en pantalla completa** de la misma app.

### 7.4 Experiencia

21. ⭐ **Tiling con configuración visual que se use con mouse o trackpad.** AeroSpace la descarta para siempre; Wimi y Tessel lo intentan pero son nuevos.
22. **Configurar sin archivos**, aunque sin cerrarle la puerta al usuario avanzado (exportar e importar configuración, CLI).
23. **Pestañas nativas bien manejadas** (nadie lo resuelve en macOS).
24. **Pantalla completa dentro del escritorio actual** para juegos y videos en Mac. Muy pedido y difícil, porque depende de cada app.
25. **Tecla Hyper configurable**, incluida la combinación de varios modificadores (`Alt+Option`).
26. **Espacios entre ventanas configurables**, que para algunos es un tema de accesibilidad.
27. **Menú de layouts en el centro de la pantalla**, no arriba (para no recorrer media pantalla con el mouse).
28. **Snap a cuartos arrastrando a las esquinas** sin elegir un layout antes.
29. **Layouts espejados** (columna angosta a la izquierda o a la derecha).
30. **Animaciones opcionales** que se puedan poner en cero.

---

## 8. Funciones nativas que casi nadie conoce

**Por qué importa:** buena parte de las quejas se resuelve con ajustes que la gente desconoce. Un gestor puede **enseñarlos o activarlos** en su bienvenida, y así ganar confianza.

### macOS
- `Cmd+Tab` y después **↑/↓**: muestra las ventanas de la app elegida.
- **"Move focus to next window"** (`Fn+Ctrl+F4` por defecto, se puede reasignar): recorre las ventanas de todas las apps.
- **`Option` + clic en el botón verde**, doble clic en la barra de título (con "Fill") o arrastrar a la barra de menú: maximiza sin pasar a pantalla completa.
- **Desactivar "Automatically rearrange Spaces based on most recent use"**.
- **Asignar una app a un escritorio:** clic derecho en el Dock → Opciones → Este escritorio.
- **`Ctrl+1…9`** para ir a un escritorio: hay que activarlo en Teclado → Atajos → Mission Control.
- **Atajos de tiling** (`Fn+Ctrl+flechas`) y atajos propios en Teclado → Atajos → Atajos de apps, usando el nombre exacto del menú Ventana.
- **`Option`** mientras se arrastra una ventana: hace snap sin llegar al borde. **`Option` + arrastrar con dirección**: "lanza" la ventana a un lado.
- **Arrastrar con tres dedos:** Accesibilidad → Control del puntero → Opciones del trackpad.
- **Hacer scroll sobre un ícono del Dock** muestra las ventanas de esa app (`defaults write com.apple.dock "scroll-to-open" -bool true`).
- **En Mission Control:** desactivar "Agrupar ventanas por aplicación", y la barra espaciadora sobre una ventana la agranda.
- **Esquinas activas** para Mission Control, App Exposé y el escritorio.
- **"Reduce motion"** para tener menos animaciones.
- **Arrastrar una ventana** y cambiar de Space con `Ctrl+flechas` la lleva al otro Space.
- **`NSWindowShouldDragOnGesture`**: mover una ventana con `Ctrl+Cmd` + arrastrar desde cualquier parte (tip del README de AeroSpace).
- **Truco para varios monitores:** desactivar "DisplayPort Deep Sleep" en el menú del monitor evita que las ventanas se muden al volver del reposo.

### Windows
- **`Win+Z`** abre el menú de layouts.
- **Configuración → Sistema → Multitarea** activa o desactiva cada parte del snap.
- **PowerToys FancyZones con "Override Windows Snap"**: `Win+flechas` entre zonas y `Win+PgUp/PgDn` dentro de una zona.

---

## 9. Perfiles de usuario

| Perfil | Necesita | Herramientas actuales | Tamaño y estado |
|---|---|---|---|
| **Viene de Windows a Mac** | Snap como el de Windows, `Alt+Tab` por ventana, vistas previas en el Dock, maximizar sin pantalla completa | Rectangle, Magnet, Wins, AltTab, DockDoor | Muy grande y muy frustrado; algunos devuelven la Mac por esto |
| **Viene de Linux con tiling** | Workspaces, teclado, layouts automáticos | AeroSpace, yabai, komorebi, GlazeWM | Ruidoso; ya resuelto a medias |
| **Monitor ultrawide o de 40"+** | Zonas fijas (centro y laterales), pilas, desplazamiento horizontal | FancyZones, Rectangle Pro, BentoBox | **Creciendo y muy desatendido**, sobre todo en Mac |
| **Varios monitores con dock** | Restaurar al conectar y desconectar y al volver del reposo, ventanas en el monitor correcto | Stay (abandonado), Display Maid (abandonado), DisplayFusion, Spencer | **Grande y muy desatendido** |
| **Usa trackpad o mouse** | Gestos, arrastrar y soltar, nada de configurar | Swish, Loop, Wins, Tessel | Grande; los tilers no lo atienden |
| **PC de trabajo administrada** | Sin permisos de administrador, licencia que permita uso comercial, portable o notarizado | GlazeWM, Rectangle | En Windows es decisivo; algunos ni pueden instalar apps |
| **Con muchas ventanas de la misma app** (Finder, navegadores, terminales) | Cambiar de ventana rápido, identificar cada una | AltTab, Witch, fswitcher | Grande |
| **Purista de Mac** | Nada; *"just learn the Mac way"* | Nativo | No es nuestro público, pero es la voz más hostil en Reddit |

---

## 10. Problemas técnicos conocidos

### 10.1 macOS

| Problema | Detalle | Cómo lo resuelven otros |
|---|---|---|
| **No hay API de gestión de ventanas** | Solo existe la Accessibility API | Todos se montan sobre ella; yabai además inyecta código en el Dock (con SIP desactivado) |
| **La Accessibility API se bloquea** | Si una app se cuelga, sus llamadas bloquean al gestor | AeroSpace #131: un hilo por app (pendiente) |
| **No se puede ocultar una sola ventana** | Solo se puede ocultar la app entera | AeroSpace la manda a una esquina y queda una franja visible. Tessel la saca de pantalla del lado sin monitor. Minimizar es lento por la animación |
| **Las ventanas no pueden salir del todo de la pantalla** | Algunas apps se redimensionan para entrar en el espacio visible | Al traerlas de vuelta, fijar posición **y** tamaño, con reintentos (Tessel) |
| **Tamaños mínimos por app** | Las apps rechazan un tamaño menor y aparecen rebotes y superposiciones | Leer el tamaño real después de cada cambio, recordar el mínimo de cada app, convertir en pestaña lo que no entra (Tessel) |
| **Pestañas nativas** | Finder, Terminal, Ghostty: cada pestaña aparece como una ventana | Nadie lo resolvió (yabai recomienda dejarlas flotando; AeroSpace #68) |
| **Spaces** | Sin API pública; el reordenamiento automático rompe los comandos | Workspaces propios (AeroSpace, Rift) o APIs privadas e inyección (yabai) |
| **Pantalla completa** | Crea un Space y cada app decide cómo lo hace | Sin solución general |
| **Secure Input** | En campos de contraseña no se pueden leer teclas, y los selectores que reemplazan a `Cmd+Tab` dejan de funcionar | fswitcher agregó soporte específico |
| **Permisos** | Accesibilidad (obligatorio); Grabación de Pantalla para vistas previas y animaciones (opcional) | Explicarlos antes de pedirlos y no volver a pedirlos si el usuario los deniega |
| **Firma de código** | Sin una firma consistente, el usuario pierde los permisos en cada actualización | Firmar siempre igual, notarizar y distribuir por Homebrew |
| **App Store** | El sandbox impide usar la Accessibility API | Distribuir fuera de la tienda (Show Bar tiene una versión recortada en la tienda) |
| **Aviso de red local** | Aparece con cualquier petición de red en apps sin sandbox | Minimizar el tráfico de red y explicarlo |
| **El notch** | Puede tapar el ícono de la barra de menú | No depender solo de la barra de menú |
| **Atajos globales** | Distinguir el modificador izquierdo del derecho | AeroSpace investiga `CGEvent.tapCreate` |
| **Ajustes del usuario** | yabai exige cambiar 4 ajustes de macOS | Detectarlos y guiar al usuario en la bienvenida |

### 10.2 Windows

| Problema | Detalle | Cómo lo resuelven otros |
|---|---|---|
| **No se puede reemplazar el gestor del sistema** | DWM no se desactiva desde Windows 8; reemplazar `explorer.exe` es mala idea | Todos usan la API pública de Win32 encima de `explorer` y DWM |
| **Escritorios virtuales** | Solo hay API privada, que puede romperse con cada versión | Wrappers (VirtualDesktopAccessor) o workspaces propios (komorebi, GlazeWM) |
| **Esconder ventanas** | — | komorebi usa el *cloaking* oculto |
| **Animaciones de minimizar y maximizar** | No se pueden desactivar | Hace que el tiling se sienta "como si alguien hiciera clic por vos" |
| **Bordes de ventana** | El color de borde vía DWM no funciona con todas las apps (Vivaldi) | GlazeWM lo combina con el color de acento del sistema |
| **Bordes invisibles** **(nota propia)** | Desde Windows 10 hay márgenes invisibles para redimensionar; `SetWindowPos` con el rectángulo crudo deja huecos | Usar `DwmGetWindowAttribute(DWMWA_EXTENDED_FRAME_BOUNDS)` |
| **Ventanas de procesos elevados** **(nota propia)** | Un proceso sin elevar no puede mover ventanas elevadas (UIPI), por ejemplo el Administrador de tareas | Avisar al usuario, ofrecer ejecutar como administrador o dejar esas ventanas flotando |
| **Barra de tareas** | Tapa las zonas si el área de trabajo se calcula mal | Usar el área de trabajo real del monitor |
| **Windows Server viejo** | APIs más antiguas | La mayoría de los tilers no funciona |

### 10.3 Ambas plataformas

- **Cambios de monitores en caliente:** conectar o desconectar, la pantalla de bloqueo con un solo monitor, despertar del reposo, monitores con distinto DPI y escala. FancyWM se colgaba, JigsawWM tuvo que corregirlo, Tiling Shell implementó su propio manejo de escala.
- **Nunca perder ventanas:** si el gestor se cae, las ventanas escondidas tienen que volver. komorebi guarda los identificadores y tiene `restore-windows`.
- **Ventanas especiales:** diálogos, paletas, popups, notificaciones, ventanas de herramientas. Hace falta una lista de reglas por defecto (por ejemplo, la calculadora, "Obtener información" y Preferencias flotando).
- **Apps que fuerzan tamaño o posición:** Zoom (pantalla completa automática), Adobe, apps con interfaz no nativa (Java, Electron con barras propias).

---

## 11. Por qué la gente desinstala en los primeros 15 minutos

### Causas observadas
1. **Avisos de permisos** sin explicación previa, o que reaparecen después de denegarlos.
2. **Desconfianza:** app sin notarizar, aviso de red local, repositorio nuevo sin licencia (el bot GitHub Guard le dio 1/6 a Wimi).
3. **Funciones que no pidió** (Show Bar).
4. **Un bug visible en la primera sesión** (Tessel: *"too buggy, uninstalled"*, aunque le gustaba la idea).
5. **No hace nada útil hasta configurarlo** (*"I can't find the files I need to edit"*, de un usuario de GlazeWM).
6. **Choca con atajos existentes** (la tecla Hyper de HyprMac contra la configuración de Karabiner del usuario).

### Lo que la bienvenida tiene que hacer
- Explicar **qué permiso** se pide y **para qué** antes de pedirlo, y dejar claro qué funciona sin cada permiso.
- **Valores por defecto útiles:** que funcione con mouse desde el primer minuto.
- **Detectar ajustes de macOS** que conviene cambiar (reordenamiento de Spaces) y ofrecer cambiarlos.
- **Detectar conflictos de atajos.**
- Un botón y un atajo de **"pausar o desactivar"** siempre visibles.
- Un **tutorial interactivo** corto (HyprMac tenía una demo interactiva en su web; fue bien recibida).

---

## 12. Mercado, precio, licencia y lanzamiento

### 12.1 Precio

| Plataforma | Qué funciona | Qué no |
|---|---|---|
| macOS | Pago único de 10–20 USD, prueba de 7–30 días sin cuenta ni tarjeta, licencia transferible, descuento de lanzamiento. Versión gratis con funciones básicas y Pro pago (modelo Wimi) | Suscripciones (sobre todo semanales); precio alto sin diferenciación clara |
| Windows | Gratis y open source; donaciones y sponsors | Licencias que impiden el uso comercial |

- MacTiler (12–20 USD) fue señalado como "franja alta" del mercado.
- Tessel gratis y sin red generó mucha confianza. fswitcher a 14,99 USD también funcionó porque el problema y la diferencia eran claros.

### 12.2 Licencia
- **MIT o GPL:** máxima adopción (AeroSpace, yabai, GlazeWM).
- **Restrictiva con uso comercial pago (komorebi):** más contribuciones según su autor, pero frena el uso en el trabajo y genera polémica sobre forks.
- **Código cerrado con promesa de abrirlo:** genera preguntas (Tessel).
- **Decisión pendiente:** si queremos usuarios en PCs de trabajo, la licencia tiene que permitirlo sin trámite.

### 12.3 Cómo lanzar (lo que funcionó y lo que no)

**Lo que funcionó (modelo fswitcher y Tessel):**
- Post estructurado: **problema → solución → comparación honesta con 2 o 3 alternativas → privacidad → precio → quién es el desarrollador y cómo contactarlo**.
- Explicar lo técnico cuando lo preguntan (Tessel explicó cómo esconde las pestañas y cómo maneja los tamaños mínimos).
- Responder rápido y publicar betas con correcciones.
- Mostrar un **video real** (a HyprMac le pidieron *"a real recording"* en vez de una animación).

**Lo que no funcionó:**
- **Nombre que choca** con un producto existente (Bento contra BentoBox).
- **Imagen generada con IA** con texto ilegible.
- **Decir que no existe nada** sin haber probado las alternativas.
- **No explicar la diferencia** con Rectangle, AeroSpace o FancyZones.
- **Ocultar el uso de IA.** Hay hostilidad (*"you didn't build shit"*, *"Claude made"*). Ayuda ser transparente y mostrar experiencia y revisión propia.

### 12.4 Confianza
- Firmado y notarizado, política de privacidad, contacto real, explicar qué datos salen de la máquina (idealmente ninguno).
- En repositorios públicos: licencia, `SECURITY.md`, más de 30 días de antigüedad, commits firmados (es lo que evalúa GitHub Guard en r/MacOS).
- Changelog activo y desarrollador visible.

### 12.5 Dónde estar
- macoswm.com (directorio con categorías: snap, tiling, híbrido, scripting).
- Planilla comunitaria de gestores de ventanas de r/macapps (Google Sheets).
- r/macapps (con reglas de autopromoción), r/MacOS ("Developer Saturday"), r/MacOSApps, r/Windows11 (pide permiso a los moderadores), r/unixporn para la versión de tiling.
- Homebrew cask; scoop y winget en Windows.

---

## 13. Principios de diseño

1. **Nunca perder una ventana.** Si el gestor se cae o se cierra, todas las ventanas vuelven a ser visibles. Hay que guardar el estado para poder recuperarlas.
2. **Siempre una salida.** Flotar una ventana, ignorar una app, desactivar en un monitor, pausar todo con un atajo.
3. **Comportamiento predecible.** La misma acción da siempre el mismo resultado; nada de estado escondido como en Snap Assist.
4. **Útil sin configurar, con profundidad para quien la quiera.** Mouse primero, teclado completo, reglas y CLI para quien las quiera.
5. **Instantáneo.** Las animaciones son opcionales y se pueden poner en cero. Nada de retrasos al llegar a los bordes.
6. **Recordar todo.** Layouts por monitor y configuración de pantallas, reaplicados solos al cambiar algo.
7. **No romper la configuración** entre versiones (la regla de komorebi).
8. **Respetar lo nativo.** Convivir con los Spaces y gestos de macOS en vez de ignorarlos; nunca pedir desactivar SIP; solo APIs públicas cuando sea posible.
9. **Una sola app enfocada en ventanas.** Sin capturas ni portapapeles; sí eventos y estado expuestos para que otros construyan encima.
10. **Transparencia.** Permisos explicados, sin telemetría o con telemetría opcional, cambios publicados.
11. **Pensado para lo que se rompe:** varios monitores, dock, reposo, distintos DPI, apps con tamaño mínimo, pestañas nativas, diálogos.
12. **Verse nativo.** Respetar el lenguaje visual de cada sistema (la crítica a AltTab).

---

## 14. La oportunidad y preguntas abiertas

### 14.1 Propuesta de valor candidata

> Un gestor de ventanas **visual**, que se use **con mouse y trackpad** (y con teclado completo), donde definís **zonas o áreas de trabajo** que pueden **apilar ventanas** o **acomodarse solas**, que **recuerda todo** (apps, monitores, dock, reposo) y lo reaplica sin que lo pidas, y que **no se rompe**.

### 14.2 Funciones candidatas, por prioridad

**Imprescindibles (MVP)**
- Zonas o áreas de trabajo definidas visualmente, con pila de ventanas por área y atajos para recorrerla.
- Arrastrar a una zona con el mouse (modificador o clic derecho), atajos por zona y menú de layouts al centro.
- Redimensionado vinculado (en los dos ejes) entre áreas vecinas.
- Reglas por app: flotar o ignorar, área preferida, tamaño mínimo y máximo.
- Persistencia: layouts por monitor y configuración de pantallas; reaplicar al conectar, desconectar o volver del reposo.
- Recuperación: nunca perder ventanas si la app se cae.
- Bienvenida con permisos explicados y desactivación rápida.

**Importantes (v1)**
- Modo tiling automático por área o por monitor, con cambio en caliente entre zonas fijas y automático.
- Grupos de apps (abrir un "contexto" con sus ventanas en su lugar).
- Selector por ventana y por monitor con posiciones estables.
- Snap Assist (llenar el espacio que queda).
- Ventanas nuevas en el monitor del puntero y sin tapar a otras.
- Activar o desactivar por monitor.

**Diferenciales (después)**
- Pestañas que mezclan apps.
- Ventana fijada en vivo.
- Vista al estilo Stage Manager (centro más miniaturas laterales).
- Desplazamiento horizontal para ultrawide.
- Eventos y estado en JSON y CLI para integraciones (barras, Raycast, Alfred).
- Importar configuraciones de otros (Rectangle, FancyZones).

### 14.3 Preguntas abiertas
1. **Plataforma:** ¿macOS, Windows o ambas? En Mac se puede cobrar y el público está más desatendido; en Windows la competencia gratuita es fuerte.
2. **Modelo técnico en macOS:** ¿workspaces propios (como AeroSpace, rápidos y robustos) o integrados a los Spaces nativos (como Glide, respetan gestos)? ¿O ambos como opción?
3. **Licencia y precio:** ¿gratis y open source, versión gratis más Pro con pago único, o licencia comercial?
4. **Alcance del MVP:** ¿zonas con pilas primero (el hueco más claro) y tiling automático después?
5. **Stack:** Swift o AppKit para Mac; Rust o C# para Windows; ¿un núcleo de layout compartido sin dependencias de plataforma (como `komorebi-scrolling`)?
6. **Nombre:** verificar que no choque con productos existentes (Bento/BentoBox, Tiles, Loop, Stage).

---

## 15. Anexo: citas destacadas

**Sobre el problema**
- *"macOS is fine for casual window placement. It falls apart when you want repeatable workspaces, saved layouts, app rules, and predictable multi-display behavior. Window managers exist because people want state, not just snapping."*
- *"Because it's app-based rather than window-based."*
- *"Try and open Finder in a multi monitor setup… You have to solve a puzzle to guess in which screen the finder will open."*
- *"The delay before the native tiling triggers when you move a window to the edge of the screen alone is a reason to use 3rd party tiling apps."*
- *"If you move a window to one side of the screen, you get prompted to choose a second window… If you dismiss that prompt… this connection isn't established… you can't accurately predict how the system will behave."* (sobre Windows)

**Sobre lo que aman**
- *"Any computer without it now feels broken somehow. Just like browsing the web without an adblocker."*
- *"Never fails. Two offices and home with different monitors… 4 monitor environments."* (Stay)
- *"I'll never go back to using two screens again."* (FancyZones en 32:9)

**Sobre lo que quieren**
- *"I figured that I don't really want auto tiling, but I want fixed workareas on each desktop."*
- *"It would be so cool to cycle between zones and automatic whenever you want!"*
- *"I don't want to configure anything. I shouldn't have to."*
- *"I want the chaos + overlapping tiling zones. I control when and IF to tile."*

**Sobre lanzar**
- *"If you want to sell a window manager in a crowded market, you need to be able to show why your product is better than the alternatives."*
- *"I think people here are tired of the thousandth iteration of the same thing."*
- *"Tabs across different apps is a new one, I haven't seen another tiler do that."*

---

## 16. Segunda ronda: YouTube, niri y el caso GlazeWM

### 16.1 Qué confirma y qué agrega

**Confirma:**
- El tiling clásico **no es para la mayoría**. El comentario más votado (1.5k): *"I wouldn't recommend a tiled wm to 90% of the people I know, for the same reason I wouldn't recommend vim."*
- **Hacen falta pocos atajos de verdad.** Un usuario de dwm desde hace años: *"there are like less than 10 keystrokes I actually needed to learn"*.
- **Instalar y configurar es donde se pierde a la gente** (ver el caso GlazeWM en §16.5).
- **Escritorios independientes por monitor** siguen siendo una razón de peso para cambiar de herramienta: *"There is a definite reason to use a tiling wm: independent virtual desktops per monitor"* (40 votos).

**Agrega:**
- **Mucha gente que "usa tiling" en realidad usa una ventana maximizada por escritorio**, y solo divide la pantalla a veces (§16.3). Es un modo de uso que ningún gestor de Mac ofrece como principal.
- **niri** es la referencia de diseño más sólida en desplazamiento horizontal y en manejo de monitores (§16.4).
- **Problemas concretos de AeroSpace** que no habían aparecido: rendimiento con CPU alta, parpadeo con monitores externos y ventanas perdidas si se cae (§16.2).

### 16.2 AeroSpace contra yabai, en los comentarios

**A favor de AeroSpace:** *"Aerospace is much better"* (61 votos), *"I can't get enough of Aerospace"*, *"simpler"*.

**En contra de AeroSpace:**
- **Rendimiento con CPU alta:** *"Aerospace has a performance issue when some high cpu usage is there. This has been a known issue for months on github."* Coincide con el ítem #131 (la Accessibility API se bloquea, ver §4.2).
- **Monitores externos:** *"Due to its method of hiding windows, it would lag the screen with flashing the windows every time you change the screen. Only happens when you have an external monitor."* Esa persona volvió a yabai. Otro usuario con dos monitores dice no tenerlo, así que **depende de la configuración**.
- **Si se cae, las ventanas quedan inutilizables:** *"When it crashes you either have a LOT of windows floating on top of each other, or they are out of reach."* Es la consecuencia de esconderlas en una esquina con un píxel visible. Refuerza el principio "nunca perder una ventana" (§13).
- **Prefieren los Spaces nativos:** *"I much rather use the native spaces."*

**A favor de yabai:**
- **Mejor soporte de mouse:** mover, redimensionar y cambiar ventanas desde cualquier parte de la ventana, no solo la barra de título. *"I hope Aerospace will improve this."*
- **Funciones únicas:** ventanas flotantes fijas en todos los Spaces, transparencia por ventana, quitar las sombras nativas.
- **El debate sobre SIP:**
  - Un usuario dice que con macOS 15 no se podían mover ventanas a otro Space sin desactivar SIP.
  - El changelog de yabai (#2788, hace ~6 meses) dice que eso **volvió a funcionar con SIP activado**.
  - **Conclusión:** depende de la versión de macOS y puede romperse con cada actualización.

**Otras posturas:**
- *"Wins is more easy to use."*
- *"Raycast pretty much covers me for everything"* (39 votos), incluso en ultrawide.
- **Raycast + Karabiner** como alternativa a instalar un gestor.
- **Riesgo de que Apple lo incorpore al sistema:** *"I'm sure Apple is going to Sherlock it by improving stage manager."*

### 16.3 Cómo usa la gente el tiling en la práctica

- **Una ventana maximizada por escritorio** (monocle / con pestañas). Es el uso más común entre los que más saben:
  - *"99% of the time I have ~8 workspaces open, each with 2-4 windows, and each window is maximized and I use Super, Shift, 0-9, and WASD (plus some Q and E…) to switch."* (usuario de Sway, 188 votos). **Es casi exactamente el esquema de `keyboard-design.md`: WASD, Q/E y números.**
  - *"I usually use monocle mode (every window is fullscreen and you flip between them) and only tile if I really need to multitask."* (usuario de dwm)
  - *"I have a bind that maximises the window, and I eventually found myself using mainly it."* (usuario de Hyprland)
  - *"One key-combination is all I need to switch from my default 'maximize everything' mode [to side-by-side], and one more to switch back."*
- **Dos ventanas por escritorio, como máximo:** *"I think most tiling window manager users only have 2 windows open on a workspace on average, but what makes it great for me is that I don't have to manually drag every window."*
- **Escritorios por número absoluto:** *"The absolute addressing of a specific desktop is what makes it super fast for me."* (con el teclado numérico)
- **El tiling ya está dentro de la terminal o del IDE:** *"tiling happens in my terminal (and neovim)"*. Los IDE completos ya dividen la pantalla internamente.
- **Desarrollo web:** *"nothing is a consistent width"*: el ancho de la app, de la terminal y del editor cambia según la tarea. El tiling rígido molesta.
- **Pantallas chicas:** *"intenté usarlos en una pantalla pequeña de laptop, se pierden muchos píxeles valiosos. Terminé siendo más productivo con GNOME lleno de extensiones."*
- **Sin configurar:** *"Yo no pierdo mi tiempo configurando mi propio tiling, para eso elijo uno que ya lo esté como Archcraft u Omarchy."* Y: *"I could get used to [Omarchy] already after the first 3 days of just casual work."* **Valores por defecto muy pensados > configuración infinita.**
- **Configurar como trampa:** *"you spend more time configuring it than actually using it"*, *"si se me pierde, se pierden más de 3 meses de arduo trabajo en configuración"*.
- **Desplazamiento horizontal:** *"the surface is effectively infinite… enables entirely logical grouping of windows"*. Pregunta abierta de un usuario: ¿cómo navegás entre ventanas que no ves? (Respuesta de niri: la vista general).

### 16.4 niri: la referencia de diseño

**Datos:** 29.1k ⭐ (a la par de yabai), **229 contribuidores**, GPL-3.0, Rust. Es un compositor Wayland completo, no una capa encima de otro sistema. Política explícita sobre el uso de LLMs en las contribuciones.

**Principios que hay que copiar:**
- *"Opening a new window never causes existing windows to resize."* Resuelve la queja #1 contra el tiling clásico (§6.6).
- *"Every monitor has its own separate window strip. Windows can never 'overflow' onto an adjacent monitor."*
- **Workspaces dinámicos, verticales e independientes por monitor**, con siempre uno vacío al final.
- *"The workspace arrangement is preserved across disconnecting and connecting monitors… upon reconnection they will move back to the original monitor."* Es **exactamente** el pedido de persistencia de §7.2.
- **Vista general** que aleja la vista para ver todas las ventanas y workspaces.
- **Gestos** de touchpad y mouse, **pestañas** para agrupar ventanas, **ventanas flotantes**.
- **Configuración que se recarga en vivo**, compatible con lectores de pantalla, y opción de **ocultar ventanas sensibles** al compartir pantalla.
- Focus-follows-mouse configurable.

**Equivalentes en otras plataformas:**
- macOS: **Paneru** y **PaperWM.spoon** (Hammerspoon).
- Linux: PaperWM (GNOME), Karousel (KDE), scroll/papersway (Sway); Hyprland tiene un layout de desplazamiento incluido.
- Windows: komorebi lo está agregando.

**Sobre Karousel y Krohnkite:** *"some apps just won't tile and stay afloat, and those I want afloat make the whole tiling thing stop working"*. Las reglas de flotar y anclar ventanas siguen siendo frágiles en todas partes.

### 16.5 Caso de estudio: instalar GlazeWM desde cero (video)

Una persona sin experiencia previa instala GlazeWM en Windows 11 (ARM) y graba todo. Es un **mapa de fricciones de la primera experiencia**:

| Minuto | Qué pasó | Lección |
|---|---|---|
| 0:00 | El problema que lo motiva: *"Every time I open a new app, it just goes wherever it wants. I'm constantly dragging things around."* | Así lo describe la gente: **"las ventanas se abren donde quieren"** |
| 1:28 | Error al abrir: falta `VCRUNTIME140.dll`. Tiene que buscar e instalar Visual C++ Redistributable para ARM64 | **Incluir las dependencias** o compilarlas dentro (nota propia: enlazar el runtime de forma estática) |
| 2:12 | Otro error: *"Windows cannot find Zebar"* (la barra que viene integrada) | Una integración opcional **no puede romper** el arranque |
| 3:27 | Redimensiona una ventana con el mouse y **vuelve sola** a su lugar | El tiling "pelea" con el usuario. Hay que respetar el cambio manual o explicar por qué no se puede |
| 4:18 | *"What's the activation key? I don't know."* | Sin mapa en pantalla, no hay forma de descubrir el modificador |
| 3:42 | Va a la web, encuentra la tabla de atajos y **descarga la imagen** para tenerla a mano | Es exactamente lo que resuelve el mapa al mantener Hyper (`keyboard-design.md` §10.1) |
| 4:36 | Arrastrar ventanas para reordenarlas sí funciona y le encanta: *"this is pretty cool"* | El mouse es la puerta de entrada |
| 5:20 | Para cambiar el modificador (`Alt` → `Ctrl`) tiene que editar el archivo de configuración y usar **buscar y reemplazar**. La documentación avisa que la tecla `Win` es difícil de usar porque el sistema reserva atajos | **El modificador tiene que poder cambiarse desde la interfaz** |
| 8:30 | Con muchas ventanas abiertas, las tiles quedan diminutas: *"I think five or four it's the maximum."* | Hace falta un **número fijo de espacios** y que el resto espere en una pila (modelo de Wimi y Ordinary Space) |
| 9:14 | Lo recomienda a *"everyone which is often using keyboard shortcuts"* y para ultrawide | El público de los tilers sigue siendo el de teclado |

**Dato extra:** GlazeWM ya tiene versión para **macOS** (aparece en la página de releases del video).

### 16.6 Lecciones para el producto

1. **Agregar un modo "una ventana maximizada por escritorio"** (monocle o con pestañas), con un atajo para pasar a dos ventanas lado a lado y volver. Es el uso real de muchos usuarios avanzados y es fácil de entender para los no técnicos.
2. **Número fijo de espacios visibles por pantalla**, y el resto apilado. Nunca tiles diminutas.
3. **Abrir una ventana nunca debería achicar las demás** sin que el usuario lo pida (principio de niri).
4. **Recuperar ventanas si el gestor se cae** es una necesidad real, no un detalle: es una de las razones por las que la gente abandona AeroSpace.
5. **Independencia del rendimiento de otras apps:** el gestor no se puede trabar cuando una app consume CPU o se cuelga.
6. **Esconder ventanas sin parpadeo** al cambiar de workspace, también con monitores externos.
7. **Mouse de primera clase**, como yabai: mover y redimensionar desde cualquier parte de la ventana con un modificador.
8. **La instalación tiene que funcionar a la primera:** sin dependencias externas, sin integraciones que fallen, con todo configurable desde la interfaz.
9. **Valores por defecto muy pensados** (el "efecto Omarchy"): la gente prefiere adoptar algo ya armado antes que configurar.
10. **Demos con apps reales** (navegador, PDF, LibreOffice, Finder), no solo terminales: *"muestren cómo es usar los programas normales"*.
11. **Respetar los cambios manuales:** si el usuario redimensiona una ventana con el mouse, no hay que devolverla a su lugar sin explicación.

---

## 17. Tercera ronda: tutoriales de yabai, AeroSpace y Raycast

**Fuentes:**
- Transcripciones de cuatro tutoriales: yabai (Josean Martinez), AeroSpace en español, AeroSpace para principiantes ("A Better Computer") y el paso de AeroSpace a Raycast (Josean Martinez).
- Comentarios de YouTube sobre esos videos y sobre la gestión de ventanas de Raycast.

### 17.1 El hallazgo principal: el recorrido de un usuario experto termina en algo simple

Josean Martinez es el autor de la guía de AeroSpace que enlaza el propio README de AeroSpace. Su recorrido:

1. **yabai + skhd**, con SIP activado. Configura unos 30 atajos: rotar, espejar, intercambiar, mover y dividir, balancear, flotar, mandar a un Space…
2. **AeroSpace**, porque no pide desactivar SIP.
3. **Lo deja por rendimiento:** *"when I had many different intensive applications running… I started experiencing a lag when switching between one Aerospace workspace to another."*
4. **Se pasa a Raycast + Hyper** y concluye:
   - *"After analyzing the way I work on my Mac I've come to realize that what I need is actually very simple."*
   - *"I would normally have only a single window open… maximized. If I ever had more than one window open it would typically be just two split vertically."*
   - Necesita solo dos cosas: **(1) atajos para abrir o ir a una app concreta** y **(2) mover y redimensionar ventanas con el teclado**.

**Su sistema final (unos 9 atajos):**

| Atajo | Acción | Lógica |
|---|---|---|
| Hyper + B / T / N / F | Ir a Chrome (**B**rowser) / **T**erminal / **N**otion / **F**inder, o abrirlos | **Inicial de la app** |
| Hyper + C | Recorrer las ventanas de la misma app (**C**ycle) | Significado. Usa el atajo nativo "Move focus to next window" reasignado |
| Option + H / L | Mitad izquierda / derecha | Posición (HJKL de Vim) |
| Option + K / J | Mitad superior / inferior | Posición |
| Option + M | **M**aximizar | Significado |

- Ya en AeroSpace usaba **workspaces con letras**: B = browser, M = música, N = notas, P = fotos, V = video, T = terminal. *"We can use mnemonics to remember where our windows are."*
- **Un comentario de otro video lo resume:** *"Window management via launcher is the best implementation imo, because I can never remember some hotkeys."*

**Conclusión:** el uso real converge en **"ir a la app por su inicial" + "mandar la ventana a una mitad o maximizar"**. Es la idea de "la letra es la palabra", pero aplicada a **apps**, no a acciones. Las iniciales de marcas (Chrome, Slack, Notion, Finder, Spotify) **no dependen del idioma**. Ver el impacto en `keyboard-design.md` §14.

### 17.2 Lo que enseñan los tutoriales sobre la curva de entrada

**Antes de usar el gestor hay que tocar ajustes de macOS** (tutorial de yabai):
- crear varios escritorios a mano en Mission Control,
- activar `Ctrl+1…9` en Atajos de teclado,
- activar "Reduce motion" para que el cambio de Space sea instantáneo,
- desactivar "Automatically rearrange Spaces".

**Instalar y configurar es trabajo de terminal:** Homebrew, `mkdir ~/.config/…`, crear el archivo con `touch`, editarlo, `brew services start`, dar permisos, reiniciar el servicio, y una segunda herramienta (skhd) con su propio archivo y permisos.

**El modelo mental es abstracto** (AeroSpace):
- árbol de contenedores, orientación horizontal o vertical, layouts "tiles" y "accordion", `join-with` para agrupar ventanas,
- un **modo "service"** (`Alt+Shift+;`) con su propio juego de atajos, al estilo Vim.
- Un principiante: *"I had no idea what i3 tiling was, so I didn't understand the concept."* Y: *"I hated it and then I kind of loved it."*
- El mismo principiante: *"This is super nerdy… Not everybody's going to want to go into a text file and have to use their terminal to reload their configuration."*

**El tiling automático se rompe enseguida:**
- *"If you bring up a 3rd app… this is completely unusable."*
- *"Some windows have kind of minimum widths that don't make them tile perfectly."*

**Funciones que a todos les gustan:**
- **El puntero sigue al foco:** al cambiar de ventana con el teclado, el puntero salta al centro de la ventana enfocada (`move-mouse window-lazy-center` en AeroSpace, `mouse_follows_focus` en yabai). *"If you're changing focus to that window it's because you're going to work there."*
- **Reglas al abrir una app:** Safari siempre a su workspace, la terminal al suyo, Notas flotando, el simulador de iPhone en tiling. *"Una de las cosas de las que más saco provecho."* Y se puede mover a mano después.
- **Mouse en yabai:** `Alt` + arrastrar = mover, `Alt` + clic derecho + arrastrar = redimensionar, soltar en el centro de otra ventana = intercambiarlas. El autor igual lo encontró *"kind of buggy"*.
- **Separar "mover" de "seguir":** mandar una ventana a otro workspace **sin** cambiar de workspace (los dos autores lo prefieren así).

**Faltan cosas que todos agregan por fuera:**
- **Indicador de foco:** AeroSpace no dibuja bordes (dice que es "estética"), pero todos los tutoriales instalan **JankyBorders** para saber qué ventana tiene el foco. **El borde de foco es una función, no un adorno.**
- **Tecla Hyper:** todos recomiendan Karabiner. Hoy **Raycast trae `Caps Lock` como Hyper incluido**: *"Karabiner Elements is no longer needed."*

### 17.3 Raycast como gestor de ventanas

**Lo que gusta:**
- **Adoptó los mismos atajos que Rectangle y Magnet** y pregunta en la bienvenida de qué app venís: *"It was both evil and genius since you, as a user, didn't have to learn anything at all."* / *"The preset applicator made it a simple switch."*
- **Su interfaz:** *"Whoever designed the UI for Raycast deserves an award"* (55 votos).
- **Layouts de ventanas con contenido:** cada parte del layout puede abrir una app **y** un enlace concreto (un sitio, un documento). *"This changes everything."*
- **Ciclos:** el mismo atajo alterna tamaños. *"I couldn't live without these commands now."*
- Funciona en ultrawide.

**Lo que piden:**
- **Agrandar o achicar la ventana un X % o X px** (*"½ is not enough, ⅔ is too much"*, Wes Bos). Raycast tiene "Make larger/smaller".
- **El divisor que redimensiona las dos ventanas a la vez**, como la pastilla blanca del tiling nativo de macOS, pero para todo el tiling.
- **Que repetir "izquierda" empuje la ventana al monitor de la izquierda**, como Windows: *"So one hotkey can intuitively 'push' windows around, I don't need to memorize too many hotkeys."* Raycast usa la repetición para cambiar de tamaño y tomó la sugerencia. **Es un conflicto de diseño** (ver `keyboard-design.md` §14.3).
- **Combinar direcciones en secuencia:** después de mandar a la derecha, "arriba" debería dejarla en el cuarto superior derecho.
- **Layouts:**
  - dos ventanas de la misma app (2× Chrome),
  - margen entre ventanas,
  - varias pestañas por ventana,
  - controlar qué ventana queda encima,
  - "todas las demás ventanas que no ubiqué a mano, mandarlas al otro monitor" (útil para compartir pantalla),
  - layouts en porcentaje para que sirvan en distintos tamaños de pantalla (existe, pero no es evidente).
- **Monitores:** en un layout con dos monitores, las apps aparecían en el principal en vez del secundario. *"When I transition from home to the office nothing is where I want it."*
- **Pantalla completa:** mover una ventana en pantalla completa a otro Space o monitor da error; si en el monitor de destino ya hay otra ventana en pantalla completa, la movida queda detrás.

**Los Spaces nativos son frágiles para todos:**
- Raycast: *"Spaces are a little bit tricky, so they're not currently supported."*
- *"Starting in Sequoia, Apple modified the private API we were using to move windows to other desktop spaces, so that feature is currently broken."* El problema también apareció en Sonoma 14.7.
- *"I was using Yabai exactly for this purpose and some Sonoma update broke it for me."*
- **Conclusión:** cualquier función que dependa de mover ventanas entre Spaces nativos **se puede romper con cada versión de macOS**. Refuerza la idea de workspaces propios o, al menos, de no depender de APIs privadas para funciones del núcleo.

**Raycast también tiene versión para Windows**, con un bug de foco: centrar con `Alt+C` solo funcionaba después de hacer clic en el escritorio.

### 17.4 Otros datos útiles

- **"Reduce motion" tiene efectos colaterales.** Es la única forma nativa de quitar la animación de cambio de Space, pero un desarrollador web avisa que afecta a todas las apps y sitios que respetan la preferencia del sistema.
  - **(nota propia)** Los navegadores exponen esa preferencia a los sitios con `prefers-reduced-motion`, así que el comentario es correcto.
  - **Cambiar de escritorio al instante sin activar "Reduce motion"** es una oportunidad (existe InstantSpaceSwitcher; yabai lo logra con SIP desactivado).
- **Mission Control se rompe con AeroSpace:** las ventanas escondidas fuera de la pantalla aparecen diminutas. El tutorial recomienda activar "Agrupar ventanas por aplicación".
- **Menos monitores, menos dolor:** *"I personally have stopped using multiple monitors… [it] helped me simplify my workflow and also reduce some neck pain."* Es el segundo comentario sobre cuello y ergonomía (el otro es el pedido de Stage Manager para Windows, §7.3).
- **Teclados personalizados:** el autor tiene una tecla que es Espacio al tocarla y Hyper al mantenerla. El mismo concepto que `Caps` = Hyper.

### 17.5 Lecciones nuevas para el producto

1. **Ir a una app por su inicial es la función más usada** por los expertos, más que acomodar ventanas. Tiene que estar en el núcleo.
2. **Importar atajos de otras apps** (Rectangle, Magnet, Raycast, AeroSpace) en la bienvenida, como hizo Raycast: "no tuve que aprender nada".
3. **Hyper incluido**, sin Karabiner.
4. **Borde o indicador de foco incluido** y prendido por defecto (es funcional).
5. **El puntero sigue al foco** como opción.
6. **Reglas por app al abrir**, creadas desde la interfaz: "cuando abro X, va a Y / flota / se ancla".
7. **Mover una ventana sin seguirla** como comportamiento por defecto, y seguirla con un modificador.
8. **Cambiar de escritorio al instante sin "Reduce motion"** y sin desactivar SIP, si es técnicamente posible.
9. **Layouts en porcentaje**, con apps **y** contenido (URL, documento), con varias ventanas de la misma app y con monitores asignados de verdad.
10. **No construir funciones del núcleo sobre APIs privadas de Spaces.**
11. **Modelo mental concreto:** zonas visibles en vez de árboles, contenedores y orientaciones.
