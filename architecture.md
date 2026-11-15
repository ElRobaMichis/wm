# Arquitectura: gestor de ventanas para Windows (Rust)

- **Estado:** propuesta, **sin código todavía**. Nada está validado: todo lo marcado **[A VERIFICAR]** se resuelve en los spikes (§13).
- **Fecha:** noviembre 2026.
- **Cómo se hizo:**
  1. Opus 5.5 hizo una propuesta inicial.
  2. **Fable 5.2** (asesor de arquitectura) hizo una propuesta **independiente**, sin ver la anterior. Leyó `findings.md` y `keyboard-design.md`, contrastó con la documentación de Microsoft y corrió una sonda en una Windows 11 real.
  3. Este documento es la versión consolidada; las diferencias entre las dos propuestas están en §2.
- **Documentos relacionados:**
  - `findings.md`: investigación de usuarios y mercado.
  - `keyboard-design.md`: modelo de interacción y mapa de atajos.

---

## Índice

0. [Etiquetas y sonda](#0-etiquetas-y-sonda)
1. [Decisiones en una página](#1-decisiones-en-una-página)
2. [Contraste entre las dos propuestas](#2-contraste-entre-las-dos-propuestas)
3. [Tecnología de interfaz](#3-tecnología-de-interfaz)
4. [Procesos e hilos](#4-procesos-e-hilos)
5. [Teclado: Caps = Hyper](#5-teclado-caps--hyper)
6. [Esconder y mostrar ventanas](#6-esconder-y-mostrar-ventanas)
7. [Fluidez a la frecuencia real del monitor](#7-fluidez-a-la-frecuencia-real-del-monitor)
8. [Geometría: DPI, bordes y monitores](#8-geometría-dpi-bordes-y-monitores)
9. [Recuperación ante caídas](#9-recuperación-ante-caídas)
10. [Pruebas](#10-pruebas)
11. [Medición y metas](#11-medición-y-metas)
12. [Crates, build, firma y distribución](#12-crates-build-firma-y-distribución)
13. [Riesgos y plan de spikes](#13-riesgos-y-plan-de-spikes)
14. [Decisiones pendientes](#14-decisiones-pendientes)
15. [Fuentes](#15-fuentes)

---

## 0. Etiquetas y sonda

Cada afirmación técnica lleva una de estas etiquetas:

| Etiqueta | Significado |
|---|---|
| **[DOC]** | Documentado por Microsoft (fuentes en §15) |
| **[SONDA]** | Medido en la máquina de desarrollo (Windows 11 Enterprise LTSC 24H2, build 26100, x64) |
| **[NO-DOC]** | Comportamiento real pero no documentado, o tomado de código de terceros |
| **[A VERIFICAR]** | Sin confirmar; se resuelve en un spike |

**La sonda:**
- Un script de PowerShell abrió una ventana de prueba, le aplicó llamadas Win32 desde otro proceso (primero sana y después con su hilo colgado) y se cerró. No dejó cambios persistentes.
- Los tiempos incluyen 6–12 ms de overhead de PowerShell, así que solo valen para saber si algo **se bloquea o no**.

| Qué se probó | Resultado [SONDA] |
|---|---|
| `DwmSetWindowAttribute(DWMWA_CLOAK)` sobre ventana de otro proceso | `E_ACCESSDENIED`: **no se puede** |
| `SetPropW` / `GetPropW` sobre ventana ajena | Funciona, también con la ventana colgada |
| `DWMWA_BORDER_COLOR`, `DWMWA_WINDOW_CORNER_PREFERENCE`, `DWMWA_TRANSITIONS_FORCEDISABLED` sobre ventana ajena | La llamada se acepta (`S_OK`); el efecto visual no se comprobó |
| `GetWindowRect` contra `DWMWA_EXTENDED_FRAME_BOUNDS` | Bordes invisibles: 7 / 0 / 7 / 7 px (izquierda / arriba / derecha / abajo) |
| `SetWindowPos` + `SWP_ASYNCWINDOWPOS` sobre ventana colgada | Vuelve al instante; la posición queda en cola |
| `SetWindowPos` **síncrono** sobre ventana colgada | **Bloqueó 8150 ms**, hasta que la app despertó |
| `GetWindowTextW`, `ShowWindowAsync`, `DwmGet/SetWindowAttribute` sobre ventana colgada | No se bloquean |
| `SendMessageTimeout(SMTO_ABORTIFHUNG, 100 ms)` sobre ventana colgada | Vuelve con 0 a los ~120 ms |
| `IsHungAppWindow` | FALSE al empezar el cuelgue; TRUE recién a los **~6 s** |

---

## 1. Decisiones en una página

1. **Núcleo sin framework de interfaz:** Win32 puro con el crate `windows`. Basado en eventos, sin bucles de consulta.
2. **Overlays** (mapa de teclado, zonas, borde de foco, fantasmas): ventanas `WS_EX_NOREDIRECTIONBITMAP` con **Windows.UI.Composition**. Direct2D/DirectWrite solo para rasterizar texto e íconos. **Toda animación la ejecuta el compositor de Windows (DWM)**, no un bucle nuestro.
3. **Ajustes:** proceso aparte, abierto a demanda, con **Slint**. Bandeja del sistema con `Shell_NotifyIconW` directo.
4. **Un ejecutable con roles:** `guardian` (mínimo), `core`, `settings` (a demanda) y modo CLI. Sin tokio. **Sin inyección de DLL:** todos los hooks son fuera de contexto.
5. **Teclado:** `WH_KEYBOARD_LL` en un hilo dedicado de prioridad máxima, con Raw Input como vigía del hook.
6. **Workspaces propios, por monitor.** No se usan los escritorios virtuales de Windows.
7. **Esconder ventanas:** cloaking del shell (`IApplicationView::SetCloak`, no documentado) **detrás de un trait**, con autotest al arrancar y **degradación automática a un método documentado**. `DWMWA_CLOAK` no sirve entre procesos.
8. **Animación de ventanas ajenas:** **instantánea por defecto**. Opcional, con un "fantasma" (`DwmRegisterThumbnail`) y un solo `SetWindowPos` por ventana.
9. **Nunca perder ventanas:** marca en la propia ventana (`SetPropW`), guardián, recuperación en cada arranque y comando `restore`.
10. **Plataformas:** Windows 11 como nivel 1; Windows 10 22H2 como nivel 2, con detección de capacidades.

---

## 2. Contraste entre las dos propuestas

Las dos coinciden en lo grueso:
- núcleo en Win32 puro con el crate `windows`,
- descartar Tauri y GPUI,
- workspaces propios,
- movimiento instantáneo por defecto y fantasmas con `DwmRegisterThumbnail` como opción,
- un proceso guardián,
- CRT estático,
- sin tokio.

Donde difieren, se adopta la versión de Fable:

| Tema | Propuesta inicial (Opus) | Propuesta de Fable | Qué se adopta y por qué |
|---|---|---|---|
| **Ventana de ajustes** | egui, a demanda | Slint, a demanda | **Slint:** texto nativo, sin la memoria base de wgpu, renderer por software. egui queda para herramientas internas. Depende de la licencia (§14) |
| **API de composición** | DirectComposition (`IDComposition*`) | Windows.UI.Composition (WinRT) | **Windows.UI.Composition:** animaciones por keyframes, implícitas y con resortes; formas vectoriales (`CompositionSpriteShape`) sin dispositivo D3D. Disponible desde Windows 10 1803 |
| **Monitores de distinta frecuencia** | "El compositor se encarga" | Los swapchains DXGI usan la cadencia del monitor **primario** [DOC] | **Prohibido renderizar overlays con swapchain**, esté o no sincronizado. Es la razón técnica por la que egui, Iced o GPUI se verían "a 60 fps" en un monitor secundario más rápido |
| **Apps colgadas** | `IsHungAppWindow` + `DeferWindowPos` | `SWP_ASYNCWINDOWPOS` + plazos propios; **prohibido** `EndDeferWindowPos` en el camino crítico | **Fable:** `IsHungAppWindow` tarda ~6 s [SONDA] y Microsoft avisa que no es de uso general; `EndDeferWindowPos` puede atarnos a la app colgada |
| **Esconder ventanas** | Abierto | Cloak del shell detrás de un trait, con autotest y respaldo documentado | **Fable:** es lo que usan komorebi y GlazeWM, y el respaldo evita que una actualización de Windows rompa el producto |
| **Riesgos no vistos en la inicial** | — | `SetForegroundWindow` restringido; Alt que abre menús; UIPI; firma de código frente a antivirus; marca con `SetPropW` para recuperar | Se suman todos (§5, §9, §12) |
| **`DCompositionWaitForCompositorClock`** | Mencionado sin detalle | Solo Windows 11; hay que resolverlo con `GetProcAddress` | Importarlo estáticamente impediría arrancar en Windows 10 |

No hizo falta una segunda ronda con Fable: su informe ya cubre y corrige cada punto de la propuesta inicial.

---

## 3. Tecnología de interfaz

**El requisito que decide:** fluidez con monitores de **distinta frecuencia**. Los swapchains DXGI usan la cadencia del monitor primario sin importar dónde esté la ventana **[DOC]**. Cualquier toolkit que redibuje cuadro a cuadro sobre un swapchain se vería entrecortado en el monitor secundario.

| Opción | Uso | Motivo |
|---|---|---|
| **Win32 + Windows.UI.Composition + D2D/DWrite** | **Overlays** | Árbol retenido; animaciones en DWM, fuera de nuestro hilo; 0 % de CPU nuestra al animar; viene con el sistema (Windows 10 1803+ [DOC]) |
| **Slint** | **Ventana de ajustes** | Liviano, declarativo, renderer por software o FemtoVG sin C++. Licencia triple: GPLv3, royalty-free con atribución visible, o comercial |
| egui | Herramientas internas | Modo inmediato, texto no nativo, memoria base de wgpu |
| Tauri | ❌ | WebView2, varios procesos, arranque lento; inviable para un overlay que aparece a los 0,4 s |
| GPUI | ❌ por ahora | Backend sólido (DX11 + DirectWrite) pero API inestable y crate oficial poco mantenido (según terceros) |
| WinUI 3 / Windows App SDK | ❌ | Rompe "un ejecutable sin dependencias"; soporte pobre desde Rust |
| Iced | ❌ | Sin ventaja sobre Slint acá |

**Diseño de overlays:**
- **Una ventana overlay por monitor**, creada una vez y oculta. **Nunca se activa:** el teclado llega por el hook, así que la app enfocada no pierde el foco.
- **Borde de foco y zonas** con `CompositionSpriteShape` (vectorial, sin dispositivo D3D).
- **El texto del mapa también es vectorial:** contornos de DirectWrite → `CompositionPath` → `ShapeVisual`, sin dispositivo D3D en nuestro proceso. D2D por hardware costó 136 ms y 55 MB de commit; D2D por software (WARP) queda de respaldo (§13.7).
- **Se precrean al arrancar** el `Compositor`, las ventanas ocultas y los targets (0,3 MB). El mapa se arma al mostrarlo, en 6–14 ms, dentro de los 0,4 s de espera de Caps.
- **El editor de zonas** es un modo del overlay del núcleo (no de Slint), para que sea idéntico a lo que se ve al arrastrar.
- **Buscador y selector de app** ("¿Qué app querés en la K?"): ventana propia activable en el núcleo, con un campo de texto mínimo por `WM_CHAR`. **Deuda:** accesibilidad (UI Automation).
- **Respetar la preferencia de animaciones del sistema** (`SPI_GETCLIENTAREAANIMATION`) **[DOC]**.
- La documentación de Composition dice que sus animaciones corren "a 60 FPS independiente del hilo de UI" **[DOC]**. **Medido en el spike 5:** corren a 144 Hz en monitores de 144 Hz (99,9 % de refrescos con posición nueva) y siguen con nuestro proceso suspendido. Falta 60 + 144 Hz y 240 Hz (§13.7).

---

## 4. Procesos e hilos

### 4.1 Procesos (un solo `.exe` con subcomandos)

| Rol | Qué hace |
|---|---|
| **guardian** | Espera sobre el handle del `core` (0 % de CPU). Si el `core` muere mal, restaura ventanas y lo relanza con backoff (máximo 3 veces por minuto) |
| **core** | Hooks, modelo, overlays, bandeja, servidor IPC |
| **settings** | Se lanza a demanda y se cierra; habla por IPC |
| **CLI** | `app.exe restore`, `app.exe state --json`, etc. |

**IPC:** named pipe con DACL del usuario, JSON por líneas, protocolo versionado.

### 4.2 Hilos del core

| Hilo | Qué hace | Regla |
|---|---|---|
| **input** | `WH_KEYBOARD_LL` y Raw Input vigía; `WH_MOUSE_LL` **solo** durante un arrastre o con Hyper apretado | `THREAD_PRIORITY_TIME_CRITICAL`. Sin asignar memoria, sin locks, sin logs: solo atómicos y una cola acotada |
| **brain** | Único dueño del estado. Bucle de mensajes; recibe WinEvents, entrada, IPC y timers | Lógica pura `(estado, evento) → efectos`. Nunca llama a nada que pueda bloquearse |
| **ops** (3 carriles: `focus`, `win`, `shell`) | Toda llamada que toca ventanas ajenas o a Explorer | Vigilado por plazo (50–300 ms según el pedido). Si vence, se abandona ese hilo y se crea otro (tope: 4 trabados). Ver §13.6 |
| **ui** | Ventanas overlay, `DispatcherQueue`, Compositor, D2D, bandeja | No espera a nadie |
| **anim** | Solo existe durante animaciones con fantasmas | Prioridad alta mientras dura |
| **io** | Configuración, caché de íconos, pipe | — |

### 4.3 Dónde corre cada hook

- **`WH_KEYBOARD_LL`** se invoca en el hilo que lo instaló, que necesita un bucle de mensajes **[DOC]**. Microsoft recomienda un hilo dedicado que delegue y retorne **[DOC]**.
- **`SetWinEventHook`** con `WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS`, en el hilo brain, registrando **rangos angostos**: foreground, movesize, minimize, create/destroy/show/hide, cloaked/uncloaked.
  - `EVENT_OBJECT_LOCATIONCHANGE` global despierta al proceso con cada movimiento del mouse: se registra **solo** para el proceso de la ventana que se está siguiendo.
- **El hook de mouse permanente** agregaría latencia a todo el sistema y rompería el 0 % de CPU; por eso es temporal.
- **Notificaciones de pantalla, energía y sesión** (`WM_DISPLAYCHANGE`, `WM_SETTINGCHANGE`, `WM_POWERBROADCAST`, `WTSRegisterSessionNotification`) van a una **ventana top-level oculta**. No puede ser message-only, porque esas no reciben broadcasts **[DOC]**.

### 4.4 Que una app colgada no nos trabe

- **Mover:** siempre `SetWindowPos` con `SWP_ASYNCWINDOWPOS`, que postea el pedido al hilo dueño de la ventana **[DOC]**. La sonda lo confirma: asíncrono vuelve al instante, síncrono se bloqueó 8 s **[SONDA]**.
- **Mostrar u ocultar:** `ShowWindowAsync`.
- **Mensajes:** solo `SendMessageTimeout` con `SMTO_ABORTIFHUNG`, y nunca desde el brain.
- **Prohibido:** `AttachThreadInput`, relaciones owner/owned con ventanas ajenas, `EndDeferWindowPos` y UI Automation en el camino crítico. Todas pueden atar nuestro hilo al de la app.
- **Plazos propios en vez de `IsHungAppWindow`:** tarda ~6 s en dar TRUE **[SONDA]** y Microsoft avisa que no es de uso general **[DOC]**.
  - Cada operación asíncrona se anota con su plazo.
  - Si no se confirma, la ventana pasa a "no responde": se trata como flotante y no se reintenta en bucle.
- **Fusión de pedidos:** si llega un rectángulo nuevo para la misma ventana, reemplaza al pendiente.
- **Ventanas `Ghost`:** las que crea el sistema para apps colgadas se ignoran.
- **Reconciliar por estado:** los eventos son pistas; ante la duda, se reescanea con `EnumWindows`.
- **CPU alta de otras apps:**
  - hilo de input a prioridad 15,
  - brain a `THREAD_PRIORITY_HIGHEST`,
  - proceso en `ABOVE_NORMAL` (no requiere admin).
  - Es seguro porque nuestro trabajo dura microsegundos.
- **0 % en reposo:** todos los hilos bloqueados en esperas; ningún timer periódico ni latido; nunca `timeBeginPeriod`.

---

## 5. Teclado: Caps = Hyper

### 5.1 Mecanismo

`WH_KEYBOARD_LL` es el único camino sin admin que permite **suprimir** teclas:
- Raw Input no suprime.
- `RegisterHotKey` no admite Caps como modificador.
- "Scancode Map" pide admin.
- Un driver de filtro pide admin y firma.

**Límites documentados del hook [DOC]:**
- El callback debe terminar antes de `LowLevelHooksTimeout`. Desde Windows 10 1709 el máximo es 1000 ms.
- Si se excede, desde Windows 7 el hook **se quita en silencio** y la app no se entera.
- Cuántos excesos hacen falta para quitarlo y cuál es el valor por defecto no están documentados: **[A VERIFICAR]**.

### 5.2 Máquina de estados (en el hilo de input, O(1))

- **Caps abajo** (scancode `0x3A`, no inyectado por nosotros): se traga y arranca un timer de 0,4 s para el mapa.
- **Otra tecla con Caps abajo:** se traga y se manda al brain con su marca de tiempo. Los acordes (W+A) y las repeticiones se resuelven en el brain, no en el hook.
- **Caps arriba sin acorde y antes del mapa:** se inyecta Caps abajo+arriba con `SendInput` y una marca en `dwExtraInfo` que el hook deja pasar. Así se alterna Bloq Mayús y su LED.
- **Invariante contra teclas trabadas:**
  - toda tecla cuyo "abajo" pasó al sistema recibe su "arriba";
  - toda tecla cuyo "abajo" se tragó también se traga su "arriba".
  - Se lleva con dos mapas de bits.
- **Eventos inyectados por terceros** (otro remapeador que genere Caps): se tratan como físicos. Solo se ignora nuestra propia marca.
- **Vuelta al reposo:** con Esc, al cambiar al escritorio seguro (`EVENT_SYSTEM_DESKTOPSWITCH`), al bloquear o desbloquear la sesión y al reinstalar el hook.
- **Posición contra significado:**
  - scancodes para posición;
  - `ToUnicodeEx` con el bit 2 de `wFlags` (no altera el estado del teclado, Windows 10 1607+) **[DOC]** para las etiquetas del mapa. Sin ese bit se corrompen las teclas muertas del usuario.

### 5.3 El Alt (pulgar)

- **Alt después de Caps:** se traga; la app nunca lo ve y no se abre ningún menú.
- **Alt antes de Caps** (un orden muy humano): la app ya vio el Alt abajo. Si al final ve Alt arriba "solo", activa la barra de menú.
  - **Solución:** inyectar una tecla máscara sin efecto antes de dejar pasar ese Alt arriba. Es la técnica de AutoHotkey (`A_MenuMaskKey`, con `vkE8` o `vkFF`) **[NO-DOC]**.
  - Lo mismo vale para la tecla Win y el menú Inicio.
- **AltGr:** el pulgar es el Alt **izquierdo** por scancode. Dentro de Hyper todo se traga, así que el Ctrl falso de AltGr no molesta.

### 5.4 Vigilancia del hook

- **Raw Input** (`RIDEV_INPUTSINK`) como testigo: si ve teclas que el hook no vio, el hook murió y se reinstala. Que Raw Input siga viendo las teclas tragadas por el hook es **[A VERIFICAR]**.
- **Reinstalación preventiva** al volver del reposo y al desbloquear.
- **Si el brain no responde** en ~2 s, el hilo de input pasa a dejar todo pasar y avisa al guardián. **El teclado nunca queda secuestrado.**
- **Si el proceso muere,** el sistema quita el hook: el fallo es benigno.

### 5.5 Ventanas elevadas (UIPI)

- Sin elevación, con una ventana elevada enfocada, **el hook no recibe teclas** **[DOC]**. Raw Input probablemente tampoco **[A VERIFICAR]**.
- Si `RegisterHotKey` dispara con una ventana elevada delante: hay reportes contradictorios **[A VERIFICAR]** (spike 1).

**Tres niveles:**
1. **Por defecto (sin admin):** detectar la ventana elevada, dejarla sin gestionar y mostrar un indicador claro ("ventana de administrador: atajos no disponibles").
2. **Opcional, UIAccess:** binario firmado, instalado en Archivos de programa, con `uiAccess="true"` en el manifiesto **[DOC]**.
   - Requiere admin una vez, al instalar. Tiene que ser un **segundo binario**: el portable no puede llevar ese manifiesto.
   - **Matiz [DOC]:** un usuario administrador arranca en integridad alta y alcanza ventanas elevadas; un usuario estándar arranca en "medium+" y **no** las alcanza.
   - Que `SetWindowPos` sobre ventanas elevadas funcione en ese modo es **[A VERIFICAR]**.
3. **Ejecutar elevado:** desaconsejado, porque las apps que lancemos heredarían la elevación.

### 5.6 Traer una app al frente (el riesgo principal)

- `SetForegroundWindow` exige, entre otras condiciones, que el proceso haya recibido **el último evento de entrada** **[DOC]**. Como nos tragamos la tecla, no lo cumplimos.
- El truco habitual es inyectar una tecla inocua con `SendInput` justo antes **[NO-DOC]**.
- Es un **riesgo de primera línea**, porque "ir a la app" es la función más usada del mapa oficial. Va al spike 1.

### 5.7 Otros límites a asumir

- No funciona en el escritorio seguro (UAC, pantalla de bloqueo).
- Clientes de RDP o VM y algunos juegos toman el teclado antes que nosotros.
- Conviene pausarse sola con apps a pantalla completa (`SHQueryUserNotificationState`).
- La prueba en vivo de la bienvenida ("apretá Caps + …") sirve también para detectar conflictos con PowerToys o AutoHotkey.

---

## 6. Esconder y mostrar ventanas

### 6.1 Workspaces propios

Los escritorios virtuales de Windows no sirven como base:
- `IVirtualDesktopManager::MoveWindowToDesktop` no funciona con ventanas de otros procesos (blog del equipo del SDK) **[DOC]**.
- No hay API pública para cambiar de escritorio ni para recibir el aviso de cambio.
- Son globales, no por monitor, que es lo que pide la gente.
- Lo demás exige interfaces privadas cuyos IID cambian entre versiones.

**Convivencia:** el escritorio virtual actual se trata como "el mundo" y se ignoran las ventanas de otros (`IsWindowOnCurrentVirtualDesktop` **[DOC]**).

### 6.2 Comparación de métodos

| Método | ¿Documentado? | Parpadeo | Efectos en la app | Si nos caemos |
|---|---|---|---|---|
| Minimizar | Sí | Animación del sistema, salvo que funcione `DWMWA_TRANSITIONS_FORCEDISABLED` | Repinta al volver; cambia la activación | **Seguro:** se recupera desde la barra de tareas |
| Fuera de pantalla | Sí | Posible con DPI mixto | Windows puede reubicarla al cambiar monitores; algunas apps guardan esa posición | Queda fuera de alcance |
| `SW_HIDE` | Sí | Repinta al volver | Apps que se confunden; desaparece de la barra de tareas | **El peor:** invisible y sin botón |
| `DWMWA_CLOAK` | Sí | — | — | **No aplica:** `E_ACCESSDENIED` entre procesos [SONDA] |
| Cloak del shell (`IApplicationView::SetCloak`) | **No** | Ninguno: DWM sigue componiendo la ventana | Ninguno conocido; sigue en la barra de tareas | Invisible hasta que alguien la recupere |

El cloak del shell es lo que usan komorebi y GlazeWM (este por defecto desde octubre de 2024). La cadena es `CLSID_ImmersiveShell` → `IApplicationViewCollection::GetViewForHwnd` → `SetCloak`, y es una llamada COM **bloqueante** hacia `explorer.exe` **[NO-DOC]**.

### 6.3 Recomendación

Cloak del shell como método primario, tratado como **capacidad opcional**:
1. **Detrás de un trait `HideStrategy`**, ejecutado en hilos ops con plazo: si Explorer está ocupado, no nos frena.
2. **Autotest al arrancar** sobre una ventana propia: cloak → leer `DWMWA_CLOAKED` (getter documentado) esperando `DWM_CLOAKED_SHELL` → uncloak. Si falla, se degrada.
3. **Lista de builds de Windows validados.** En un build desconocido, solo se usa si pasa el autotest.
4. **Respaldo documentado:**
   - primera opción, **minimizar con las transiciones desactivadas**, el único método seguro por construcción. La llamada se acepta entre procesos **[SONDA]**; que quite la animación es **[A VERIFICAR]**;
   - segunda opción, fuera de pantalla.

**Resultado:** si una actualización de Windows rompe la interfaz privada, la experiencia empeora un poco (algo de parpadeo) pero **el producto no se rompe**. Es la diferencia con los gestores que dependen de APIs privadas para funciones del núcleo, que es lo que `findings.md` pide evitar.

**Problemas conocidos del cloak, a cubrir desde el diseño:**
- **Minimizado nativo:** GlazeWM tiene un bug abierto donde minimizar con el botón nativo deja ventanas desaparecidas (#1358).
- **Caída:** forks de GlazeWM documentan ventanas varadas tras matar el proceso (ver §9).
- **Foco:** una ventana cloaked puede conservar el foco de teclado; hay que moverlo antes de esconderla.
- **Cambio de escritorio virtual nativo:** el shell puede descloakear lo nuestro; hay que escuchar `EVENT_OBJECT_UNCLOAKED` y reafirmar.
- **Reinicio de Explorer** con ventanas cloaked: comportamiento desconocido **[A VERIFICAR]**.

**Orden al cambiar de workspace:**
1. mostrar el destino,
2. esconder el origen,
3. mover el foco.

Así no se ve el fondo de pantalla entre medio. Las ventanas escondidas se mantienen ya posicionadas.

**Barra de tareas:** todas las ventanas quedan visibles ahí. Si el usuario hace clic en una escondida, llega el evento de foreground y saltamos a su workspace. Es coherente con "nunca perder una ventana".

**Pilas de una zona:** usan el mismo `HideStrategy`. En modo degradado, simplemente quedan detrás por orden Z.

---

## 7. Fluidez a la frecuencia real del monitor

### 7.1 Qué controla quién

- **Windows 11:** el reloj del compositor "no necesariamente sale del monitor primario" y hay estadísticas por pantalla **[DOC]**. `DCompositionWaitForCompositorClock` existe desde el build 22000 **[DOC]**.
- **Windows 10 2004+:** DWM compone al ritmo del monitor más rápido, con salvedades reportadas (por ejemplo 240 + 60 Hz) **[NO-DOC]**.
- **Consecuencia:** el techo lo pone DWM. **Nuestro trabajo es no ser nunca el factor limitante.**

### 7.2 Overlays

- **Todas las animaciones son del compositor** (`KeyFrameAnimation`, implícitas, resortes): aparición del mapa, cambio de capa al apretar el pulgar, resaltado de zonas, borde de foco.
- **Nuestro proceso emite la animación y se duerme:** 0 % de CPU y nada que planificar a 240 Hz.
- **Sin swapchains**, ni siquiera con espera: heredan la cadencia del primario **[DOC]**.
- **Una ventana overlay por monitor**, para que cada una viva en una sola pantalla y un solo DPI.

### 7.3 Borde de foco

- **Windows 11, opción liviana:** `DWMWA_BORDER_COLOR` **[DOC]**, aceptado entre procesos **[SONDA]**. Lo dibuja DWM junto con el marco: sincronía perfecta y costo cero. Limitaciones: es fino y no afecta a ventanas sin marco estándar.
- **Borde propio** (overlay, más grueso y animable):
  - los WinEvents llegan tarde, así que durante un arrastre del usuario el borde iría un cuadro atrás;
  - lo honesto es **ocultarlo** entre `EVENT_SYSTEM_MOVESIZESTART` y `EVENT_SYSTEM_MOVESIZEEND`;
  - **nunca** hacerlo "owned" de la ventana ajena (ata las colas de entrada);
  - **ocultarlo con apps a pantalla completa:** un overlay encima puede forzar composición y afectar a los juegos **[A VERIFICAR]**.

### 7.4 Mover ventanas de otras apps

| Técnica | Evaluación |
|---|---|
| `SetWindowPos` por cuadro | Mover sin redimensionar es barato. **Redimensionar obliga a la app a repintar a su ritmo:** imposible a 240 Hz con Electron o navegadores. Frágil bajo carga |
| **Fantasma con `DwmRegisterThumbnail`** | Documentado; el destino tiene que ser una ventana nuestra [DOC]. Se esconde la real, se mueve **una sola vez** y se anima la miniatura en vivo con `DwmUpdateThumbnailProperties` por cuadro desde el hilo `anim` |
| `DwmFlush` | Marcapasos en Windows 10 [DOC] |
| `DCompositionWaitForCompositorClock` | Marcapasos en Windows 11. **Resolver con `GetProcAddress`**, para poder arrancar en Windows 10 |
| `DwmpCreateSharedThumbnailVisual` (ordinal 147) | Fantasma animado por el compositor, pero privado [NO-DOC]. Solo experimento |
| `IDCompositionDevice::CreateSurfaceFromHwnd` | Documentado para ventanas layered [DOC]; que incluya la miniatura es [A VERIFICAR]. Solo experimento |

**Recomendación:**
- **Por defecto, movimiento instantáneo.**
- **Animación opcional con fantasma**, en esta secuencia sin destellos:
  1. fantasma encima de la ventana real, en el rectángulo viejo;
  2. cloak de la real;
  3. un solo `SetWindowPos` al destino;
  4. animar el fantasma;
  5. uncloak de la real;
  6. quitar el fantasma.
- **La posición se calcula por tiempo**, no por número de cuadro, para que sirva a cualquier frecuencia.
- **El fantasma depende del cloak:** en modo degradado, las animaciones de ventanas se apagan o quedan en solo-mover.
- **Verificado en el spike 6:** la miniatura de una ventana cloaked sigue en vivo. Con esta secuencia, 0 cuadros sin ventana o con doble imagen en Brave, Windows Terminal y títeres, también con la CPU al 100 % (§13.8).

---

## 8. Geometría: DPI, bordes y monitores

- **DPI:** manifiesto `PerMonitorV2`; todo en píxeles físicos de la pantalla virtual.
- **Bordes invisibles:** margen = `GetWindowRect` − `DWMWA_EXTENDED_FRAME_BOUNDS` **[DOC]** (7/0/7/7 px en la sonda **[SONDA]**). Se calculan **por ventana y por DPI**, nunca fijos, y se suman al rectángulo pedido.
- **Cruce a un monitor con otro DPI:** la app recibe `WM_DPICHANGED` y se redimensiona sola. Hay que aplicar, leer el resultado y reaplicar una vez (otros tilers hacen doble `SetWindowPos` **[NO-DOC]**).
- **Área de trabajo:** `GetMonitorInfoW().rcWork` **[DOC]**; recalcular con `WM_SETTINGCHANGE` (`SPI_SETWORKAREA`).
- **Tamaños mínimos:** enfoque empírico.
  - Pedir el tamaño, leer el rectángulo real y recordar el mínimo por app.
  - Resolver el layout de nuevo como máximo una o dos veces, **nunca en bucle**. Evita los "rebotes" de `findings.md` §6.4.
- **"Full":** rectángulo igual al área de trabajo con compensación de bordes, no `SW_MAXIMIZE`, para evitar la animación del sistema.
- **Esquinas redondeadas de Windows 11:** `DWMWA_WINDOW_CORNER_PREFERENCE` **[DOC]**, aceptado entre procesos **[SONDA]**, permite cuadrarlas en ventanas en mosaico.
- **Redondeo:** calcular **bordes compartidos**, no anchos, para no dejar huecos de 1 px.
- **Identidad de monitores:** `QueryDisplayConfig` + `DisplayConfigGetDeviceInfo` + número de serie del EDID: la tupla EDID + serie + conector, emparejada por niveles. No `\\.\DISPLAYn`, `HMONITOR` ni el LUID del adaptador (§13.9).
- **Tormentas de cambios de pantalla:** esperar a que se calmen (300–500 ms) antes de reaplicar. Windows 11 también recuerda posiciones por monitor y mueve ventanas por su cuenta: hay que reconciliar después, no pelear en el momento.

---

## 9. Recuperación ante caídas

Defensa en capas, de adentro hacia afuera:

1. **La verdad vive en la ventana.** Antes de esconderla, `SetPropW(hwnd, marca, workspace)`. Funciona entre procesos, incluso con la ventana colgada **[SONDA]**. Recuperar es `EnumWindows` + `GetPropW`: no depende de nuestra memoria ni de un archivo.
2. **Diario en disco** como segunda red: HWND + PID + hora de creación del proceso, para no tocar un HWND reciclado.
3. **Pánico en Rust:** `catch_unwind` en el borde del bucle; restaurar todo con plazo, salir, y que el guardián relance.
4. **Guardián:** si el core muere sin marca de salida limpia, muestra todo lo marcado y lo relanza (máximo 3 veces por minuto).
5. **Arranque = recuperación.** Cada inicio escanea y restaura lo marcado antes de gestionar nada. Así ese camino se ejercita siempre.
6. **Comando y menú "Mostrar todas las ventanas"** (`app.exe restore`). El actualizador lo ejecuta antes de reemplazar el binario.
7. **Salidas limpias** (pausar, salir, desinstalar): siempre muestran todo.
8. **Monitores:** tras cada cambio de pantallas, toda ventana gestionada tiene que intersectar un área de trabajo; si no, se trae de vuelta.
9. **Cuelgue propio:** lo detecta el hilo de input (§5.4) y lo resuelve el guardián.
10. **Volcado local de fallos** (`MiniDumpWriteDump`), sin telemetría.

**Caso no cubierto en caliente:** que maten el core y el guardián a la vez con ventanas cloaked. Se recupera en el próximo arranque o con `restore`.

---

## 10. Pruebas

- **Núcleo puro** (crate `wm-model`, sin dependencia de `windows`, con `#![forbid(unsafe_code)]`): layout, zonas, pilas, deshacer y la máquina de estados del teclado. Corre en cualquier sistema operativo, en milisegundos.
- **Propiedades** (`proptest`, con máquina de estados):
  - ninguna ventana se pierde (cada una está en exactamente un lugar);
  - los rectángulos quedan dentro del área de trabajo y no se solapan;
  - deshacer(hacer(x)) = identidad;
  - desconectar y reconectar un monitor devuelve las ventanas a su lugar;
  - teclado: para cualquier secuencia, ninguna tecla queda trabada, y sin Hyper la máquina es la identidad.
- **Simulación determinista:** una plataforma falsa con semilla y apps "difíciles" (tamaño mínimo, colgada, que se reubica sola, que muere a mitad de una operación, HWND reciclado).
- **Instantáneas** (`insta`): layouts, y **cualquier versión histórica de la configuración tiene que seguir cargando** (regla de no romper la configuración).
- **Integración con ventanas reales:**
  - un ejecutable **"títere"** configurable: tamaño mínimo, se cuelga a pedido, DPI-unaware, sin borde, popup con dueño;
  - se verifica con `GetWindowRect`, `DWMWA_EXTENDED_FRAME_BOUNDS` y `DWMWA_CLOAKED`;
  - la entrada se inyecta con `SendInput`.
- **Caos:** matar el core en momentos aleatorios durante cambios de workspace; matar Explorer; colgar el títere.
- **Resistencia:** 24 h de operaciones aleatorias vigilando handles, objetos GDI/USER y bytes privados.
- **CI:** GitHub Actions `windows-latest` y `windows-11-arm`. Si esos runners permiten hooks, DWM y cloaking es **[A VERIFICAR]** (spike 9).
  - Tienen un solo monitor a 96 DPI, así que multi-monitor, DPI mixto y frecuencias van a **una máquina propia con dos pantallas** y a una lista de pruebas manual por versión.
- **Instalación limpia:** Windows Sandbox o una VM sin Visual C++ Redistributable.

---

## 11. Medición y metas

| Meta | Herramienta | Qué mirar |
|---|---|---|
| CPU en reposo = 0,0 % | Windows Performance Recorder + Analyzer (CPU Usage Precise); Process Explorer | Cambios de contexto y ciclos = 0 durante 10 min sin eventos; `powercfg /energy` sin pedidos de resolución de timer |
| RAM: ≤ 15 MB antes del primer overlay, ≤ 30 MB después (working set privado, **a validar**) | VMMap; "Working Set - Private" y "Private Bytes" | Medir también el delta de `dwm.exe` y la memoria de GPU, porque las superficies viven ahí |
| Latencia del hook: p99.9 < 200 µs, máximo < 5 ms | `QueryPerformanceCounter` + histograma (`hdrhistogram`); eventos ETW (`tracelogging`) | p50, p99, p99.9 y máximo, con y sin carga (CPUSTRES de Sysinternals) |
| Tecla → acción visible < 1 frame | Marcas de tiempo internas; cámara lenta del teléfono (240/960 fps) filmando tecla y pantalla | De punta a punta |
| Cuadros: ≥ 97 % de refrescos con posición nueva en cada monitor | PresentMon sobre `dwm.exe`; `DCompositionGetTargetStatistics` (Windows 11); Desktop Duplication contando posiciones distintas | Por monitor, en 60 + 144/240 Hz |
| Fugas y fallos | Application Verifier; volcados | — |

**Prueba clave de "lo anima el compositor":** suspender nuestro proceso a mitad de una animación de overlay y ver que la animación termina igual.

---

## 12. Crates, build, firma y distribución

### 12.1 Crates y build

- **`windows`, no `windows-sys`:** hace falta COM y WinRT (Composition, D2D, DWrite, shell) y declarar interfaces privadas con `#[interface]`. **Fijar versiones exactas** (Fable reporta la línea 0.62.x con un salto anunciado a 0.100 **[A VERIFICAR]**).
- **CRT estático:** `-C target-feature=+crt-static` para `x86_64-pc-windows-msvc` y `aarch64-pc-windows-msvc`.
  - Elimina el error de `VCRUNTIME140.dll` del video de GlazeWM.
  - **Control en CI:** `dumpbin /dependents` solo puede listar DLL del sistema.
- **Estado del spike 0 (noviembre 2026), en la máquina de desarrollo:**
  - El toolchain instalado es **GNU** (`x86_64-pc-windows-gnu`, con el linker de WinLibs UCRT), no MSVC. Sirve para desarrollar, con dos diferencias:
    - el ejecutable importa `api-ms-win-crt-*` (el UCRT, que viene con Windows 10/11, así que no hace falta redistributable);
    - el linker avisa "multiple non-default manifests", aunque el DPI `PerMonitorV2` se aplica igual (verificado con `wm doctor`).
  - **El release y el CI usan MSVC con `crt-static`**, donde ni `vcruntime` ni `api-ms-win-crt` deben aparecer: el CI lo exige con `dumpbin /dependents`.
  - Para desarrollar con el mismo toolchain que el release hay que instalar Visual Studio Build Tools (C++ y Windows SDK) y `rustup toolchain install stable-x86_64-pc-windows-msvc`.
- **Sin C++:** en Slint, renderer por software o FemtoVG, no Skia.
- **Manifiesto embebido** (`embed-manifest`): `PerMonitorV2`, `asInvoker`, `supportedOS`, Common Controls v6. La variante UIAccess es un segundo artefacto.
- **Otras dependencias:**
  - `serde` + `toml_edit` (la interfaz escribe la configuración sin destruir los comentarios del usuario),
  - `serde_json`, `crossbeam-channel`, `tracing` (con rotación de logs),
  - `proptest`, `insta`, `criterion` para pruebas.
  - Asignador del sistema; sin tokio.
- **Perfil de release:** `lto`, `codegen-units = 1`, `panic = "unwind"` (para poder restaurar ventanas), PDB archivados.
- **ARM64:** `aarch64-pc-windows-msvc` es Tier 1 desde Rust 1.91 **[A VERIFICAR versión]**.
  - Se publican **dos ejecutables nativos** (no existe binario universal).
  - Como no inyectamos DLL, un solo proceso nativo gestiona apps x86, x64 y ARM64.

### 12.2 Estructura de crates propuesta

```
window-managment/
├─ crates/
│  ├─ wm-model/     # núcleo puro: layout, zonas, pilas, deshacer, teclado (sin `windows`, sin unsafe)
│  ├─ wm-platform/  # Win32: hooks, WinEvents, SetWindowPos, HideStrategy, DPI, monitores
│  ├─ wm-overlay/   # Windows.UI.Composition + D2D/DWrite: mapa, zonas, borde, fantasmas
│  ├─ wm-ipc/       # protocolo versionado (named pipe, JSON por líneas)
│  └─ wm-settings/  # Slint (proceso a demanda)
├─ app/             # binario único: guardian | core | settings | CLI
└─ tools/
   └─ puppet/       # ventana "títere" para pruebas de integración
```

### 12.3 Firma de código

**Sin firma**, un binario Rust con un hook de teclado choca con SmartScreen y con las heurísticas de los antivirus (**parece un keylogger**). El nivel UIAccess exige firma sí o sí.

| Opción | Costo | Disponible para | SmartScreen |
|---|---|---|---|
| Azure Artifact Signing | ~10 USD/mes | Individuos: **solo EE. UU. y Canadá** | Reputación gradual |
| Certificado OV | 150–300 USD/año, clave en HSM | Todo el mundo | Reputación gradual |
| Certificado EV | 400+ USD/año | Todo el mundo | Igual que OV desde 2024: ya no vale la pena |
| **SignPath Foundation** ✅ elegida (el proyecto es de código abierto) | **Gratis** | **Proyectos de código abierto** | Reputación gradual |
| Microsoft Store (MSIX) | Gratis | Todo el mundo | Sin avisos; compatibilidad con hooks y UIAccess **[A VERIFICAR]** |

(Datos según la página de Microsoft que consultó Fable; ver §15).

### 12.4 Distribución

- Ejecutable portable en GitHub Releases.
- scoop (solo hash, ideal para portable) y winget (tipo `portable`, **[A VERIFICAR]** contra el esquema vigente).
- Instalador por máquina **solo** para la variante UIAccess.
- Inicio automático por `HKCU\...\Run` (sin admin).
- Autoactualización opcional y **sin tráfico de red por defecto**.

---

## 13. Riesgos y plan de spikes

### 13.1 Riesgos principales

1. **Fragilidad del hook:** se quita en silencio, UIPI, juegos, otros remapeadores, antivirus.
2. **Foco:** `SetForegroundWindow` desde una tecla tragada depende de un truco no documentado, **y es la función más usada**.
3. **Esconder sin parpadeo depende de una interfaz privada;** las alternativas documentadas tienen costos visibles.
4. **240 Hz reales en ventanas ajenas no dependen de nosotros:** redimensionar exige que la app repinte, y el fantasma depende del cloak.
5. **Ventanas elevadas sin admin:** agujero de experiencia; UIAccess pide firma e instalación con admin.
6. **Diversidad de apps:** qué ventana es "gestionable", UWP dentro de `ApplicationFrameHost`, Electron sin marco, apps que se reubican solas.
7. **Firma y antivirus.**
8. **Monitores:** identidad estable, tormentas de eventos, Windows moviendo ventanas por su cuenta.
9. **Metas de RAM** frente al costo del dispositivo D3D.
10. **Alcance de la interfaz propia:** escena de Composition, entrada de texto y accesibilidad son ingeniería a medida.

### 13.2 Spikes, en orden

Los spikes 1 y 2 son independientes, se pueden hacer en paralelo y son los que más condicionan todo lo demás.

| # | Spike | Criterios de éxito | Qué decide |
|---|---|---|---|
| 0 | Esqueleto, CI, CRT estático (1 día) | Arranca en Windows 10 22H2 x64 y Windows 11 ARM64 limpios, sin redistributables; `dumpbin` solo lista DLL del sistema | Base de build |
| 1 | **Caps = Hyper + ir a la app** | Callback p99.9 < 200 µs y máximo < 5 ms con CPU al 100 % durante 10 min; 0 hooks perdidos en 24 h; con una pausa inducida, se detecta y reinstala en < 1 s; 0 teclas trabadas en 10.000 acordes aleatorios; Alt-antes-de-Caps no abre menús en Notepad, Explorer, Office, Chrome y VS Code; **`SetForegroundWindow` funciona 100/100**; documentado qué ven hook, Raw Input y `RegisterHotKey` con el Administrador de tareas enfocado; variante UIAccess probada; 0 cambios de contexto en reposo | Viabilidad del concepto "Apps primero" y del nivel UIAccess |
| 2 | **Esconder y mostrar** | Las cuatro estrategias detrás del trait; con 10 ventanas por workspace, 0 cuadros con fondo visible y cambio p95 < 30 ms; matriz de apps (Chrome, Firefox, Electron, Office, Explorer, Terminal, UWP, WPF, Java, Qt, Teams/Zoom) sin efectos; comportamiento documentado al matar Explorer y al cambiar de escritorio virtual; si `DWMWA_TRANSITIONS_FORCEDISABLED` quita la animación de minimizar | Método primario y respaldo |
| 3 | Seguridad ante caídas (junto con el 2) | 1000 muertes aleatorias del core: 100 % de ventanas visibles y en pantalla en < 500 ms; matando ambos procesos, 100 % al siguiente arranque; ningún HWND reciclado tocado | Diseño del guardián |
| 4 | Apps colgadas y CPU alta | Con 1 ventana colgada entre 10, las demás operaciones p99 < 50 ms; el brain nunca detenido > 5 ms; con CPU al 100 %, tecla → pedido emitido p99 < 10 ms; ninguna llamada usada se bloquea | Modelo de hilos ops |
| 5 | **Overlays a la frecuencia real** | En 60 + 144/240 Hz, ≥ 97 % de cuadros con posición distinta en cada monitor; la animación termina con nuestro proceso suspendido; primer cuadro del mapa < 50 ms en frío; RAM dentro de las metas; funciona en Windows 10 22H2 | Pila de overlays |
| 6 | Animación de ventanas | Fantasma a 240 Hz en VS Code, Chrome y Explorer: ≥ 95 % de cuadros distintos, 0 cuadros con doble imagen o sin ventana, un solo `SetWindowPos`. **Si no se cumple, se lanza sin animación de ventanas** | Si la función existe |
| 7 | Geometría y pantallas | Error del marco visible ≤ 1 px a 100–200 % de escala; cruce de DPI estable en ≤ 2 pasos; 50/50 ciclos de conectar, reposo y bloqueo sin ventanas fuera de pantalla | Motor de persistencia |
| 8 | Ajustes: Slint contra alternativa | Misma pantalla en ambos: arranque < 300 ms, RAM, tamaño, Narrador, traducción | Toolkit de ajustes |
| 9 | CI con ventanas reales | Hooks, `SendInput`, DWM y cloaking funcionan en runners x64 y ARM64, o se define qué va a máquina propia | Estrategia de CI |

### 13.3 Estado del spike 1 (noviembre 2026)

**Implementado:**
- Máquina de estados de Hyper en `wm-model::keyboard` (pura). Cubre:
  - toque corto = Bloq Mayús inyectado;
  - acordes y repeticiones;
  - pulgar (Alt izquierdo) antes o después de Caps;
  - tecla máscara al soltar Alt/Win apretados antes de Caps.
  - **10.000 secuencias aleatorias sin teclas trabadas ni "arriba" huérfanos.**
- Hook `WH_KEYBOARD_LL` en un hilo `TIME_CRITICAL` (`wm-platform::input`): sin asignar memoria, sin locks, avisa al brain con `PostThreadMessageW`, histograma de latencia en atómicos.
- Capa de apps (`wm-model::apps`, pura y probada): asignaciones en `%APPDATA%\wm\apps.txt`, modo automático por inicial del ejecutable, repetir = siguiente ventana de la app, abrir la app si está cerrada.
- Traer al frente con `SetForegroundWindow` y, si falla, la tecla neutra marcada (§5.6). El log de `wm core` dice cuándo hizo falta.
- `wm core` (prototipo) y `wm apps` (diagnóstico sin efectos). Enumerar 5 ventanas: 0,4 ms.

**Primera prueba real (15/11/2026, Windows 11 24H2, 2 monitores a 144 Hz):**

| Medición | Resultado | Meta |
|---|---|---|
| Traer al frente (`Caps` + B/C/W, 8 veces) | **8/8 ok**. Solo la **primera** necesitó la tecla neutra; después Windows dejó cambiar el foco directo | 100/100 |
| Tiempo de activación | 1,4–2,4 ms; total con enumeración ≤ 3 ms | < 1 cuadro (6,9 ms a 144 Hz) ✅ |
| Latencia del hook (358 eventos) | p50 < 2 µs, p99 < 32 µs, **máx 875 µs** | p99.9 < 200 µs, máx < 5 ms |
| `Caps` + Alt + B (Alt antes de Caps) | Detectado como capa de ventana (pulgar), como corresponde | — |
| **Máscara de Alt** en el Bloc de notas (Alt → Caps → B, soltar todo) | **No se abrió ningún menú** ✅. Falta repetirlo en Explorer, Office, Chrome y VS Code | Sin menús |
| Toque de `Caps` = Bloq Mayús; mantener > 0,5 s no lo cambia; escritura normal con Shift, acentos y Ñ | ✅ Todo como se esperaba | — |

Lectura de la latencia:
- **El máximo está dentro del límite** de 5 ms y muy lejos del timeout de Windows (1000 ms).
- **El p99.9 no se puede evaluar todavía:** con 358 eventos el p99.9 es prácticamente el máximo, y el histograma por potencias de 2 solo dice "< 1024 µs".
- **Sospecha:** los picos vienen de llamar a `SendInput` **dentro** del callback (el Bloq Mayús inyectado o la máscara de Alt).
- **Siguiente paso:**
  1. histograma más fino;
  2. separar la latencia de los eventos con inyección y sin ella;
  3. si se confirma, mover la inyección a otro hilo, cuidando el orden de la máscara de Alt.

**Hallazgo:** el nombre del ejecutable no siempre es la "inicial natural" de la app. Windows Terminal es `windowsterminal` (W, no T) y Steam es `steamwebhelper`. Para la asignación automática conviene usar el nombre visible (FileDescription del ejecutable o el AppUserModelID), no el nombre del archivo.

**Segunda iteración (cierre del spike), implementado:**
- **Vigía con Raw Input:**
  - una ventana oculta del hilo de entrada recibe las teclas físicas por Raw Input (`RIDEV_INPUTSINK`);
  - si llegan 3 seguidas que el hook no vio, se reinstala (como máximo una vez cada 5 s);
  - la comparación usa solo teclas "abajo" y de dispositivos reales (las inyectadas no cuentan).
- **Reinstalación preventiva** al volver del reposo (`WM_POWERBROADCAST`) y al desbloquear la sesión (`WTSRegisterSessionNotification`). Al bloquear, la máquina de estados vuelve a reposo.
- **Modo seguro:**
  - si una acción lleva más de 2 s sin respuesta del brain, el hook deja pasar todo (`Caps` vuelve a ser Bloq Mayús);
  - sale solo cuando el brain responde;
  - se implementa con atómicos, sin timers.
- **Latencia con resolución de 10 µs**, separada en eventos con y sin `SendInput`.
- **Contadores** de activaciones, uso de la tecla neutra, reinstalaciones y entradas en modo seguro, impresos al salir.
- **Herramientas de prueba:**
  - `WM_TEST=1` habilita `Caps` + F11 (traba el brain 6 s) y `Caps` + F12 (traba el hook 1,5 s);
  - `wm burn [s]` ocupa todos los núcleos al 100 %.

**Segunda prueba real (15/11/2026, con `wm burn` al 100 % de CPU):**

| Medición | Resultado |
|---|---|
| Traer al frente | **64/64 ok**, 1 sola vez con tecla neutra; 1,3–4,8 ms |
| Latencia sin inyección (1338 eventos) | p50 ≤ 10 µs, **p99 ≤ 20 µs** ✅ (el máximo de 1,5 s son las trabas a propósito de F12) |
| Latencia **con** inyección (9 eventos) | **~0,85 ms**: `SendInput` dentro del callback es caro → se movió a un hilo aparte |
| Desbloquear la sesión | Hook reinstalado solo ✅ |
| Ventana de administrador al frente (Administrador de tareas) | El hook **no recibe** teclas (UIPI, como se esperaba): `Caps` funciona como Bloq Mayús nativo y `Caps` + B no hace nada. El vigía no reinstaló (no hubo falsos positivos), aunque con solo 3 teclas la prueba no es concluyente |
| `Caps` + F12 (callback trabado 1,5 s, 3 veces) | **Windows no quitó el hook.** Pero **bug:** durante la traba, Windows dejó pasar eventos sin consultar al hook (incluido soltar Caps). La máquina de estados creyó que Caps seguía apretado y cada letra se tomó como `Caps` + letra, hasta volver a tocar Caps |
| `Caps` + F11 (brain trabado 6 s) | **No concluyente:** el modo seguro no se activó (0 entradas). Probablemente Caps se tocó antes de los 2 s. Además, Bloq Mayús funciona aunque el brain esté trabado (lo maneja el hook), así que no servía como señal |

**Correcciones después de esta prueba:**
- **Callback lento (> 100 ms):** la máquina de estados vuelve a reposo y se avisa. Cubre cualquier causa de eventos perdidos durante una traba.
- **Inyección en un hilo aparte** (`wm-inject`): el callback solo postea un mensaje.
- **Acciones viejas descartadas:** si una acción llega al brain más de 1 s después de la tecla, no se ejecuta (evita que `Caps` + B apretado durante una traba te lleve a Brave segundos después).
- **El modo seguro informa cuánto duró.** La prueba se rediseñó: durante la traba, `Caps` + B tiene que **escribir** una B en vez de ir a Brave.
- Las trabas de prueba no se cuentan en la latencia.

**Tercera prueba real y cierre (15/11/2026):**

| Medición | Resultado | Meta |
|---|---|---|
| Modo seguro (`Caps` + F11, `Caps` + B pasados 2 s) | Se escribió la B; modo seguro 2,2 s; el programa avisó al volver ✅ | El teclado nunca queda secuestrado |
| Callback trabado (`Caps` + F12) | Estado reiniciado; las letras salieron normales ✅ | Sin Caps trabado |
| Latencia sin inyección (780 eventos) | p50 ≤ 10 µs, p99.9 ≤ 20 µs, **máx 19,5 µs** ✅ | p99.9 < 200 µs, máx < 5 ms |
| Latencia con inyección (ahora en `wm-inject`) | **11,5 µs** (antes 864 µs) ✅ | — |
| Traer al frente (acumulado de las tres pruebas) | **80/80 ok**; tecla neutra solo en la primera activación de cada sesión | 100/100 |
| **Reposo** (30 s sin teclas, medido con `Get-Process`) | **0,0 ms de CPU**, 3 hilos, **working set 8,9 MB, memoria privada 1,4 MB**, 113 handles ✅ | 0 % de CPU, < 20 MB |

**✅ Spike 1 cerrado:** el concepto "Apps primero" (`Caps` + inicial) es viable en Windows sin permisos de administrador.

**Queda como deuda para la etapa de resistencia** (no bloquea el spike 2):
- **El vigía nunca vio un hook realmente quitado:** en 3 trabas de 1,5 s, Windows no lo quitó. La lógica está, pero no se pudo provocar la situación real.
- **Máscara de Alt** en Explorer, Office, Chrome y VS Code (verificada solo en el Bloc de notas).
- **Prueba de 24 h** y conteo de cambios de contexto en reposo con WPR.
- **Ventanas elevadas:** comportamiento documentado (el hook no las ve). UIAccess queda postergado (§14).
- **Asignación automática por nombre visible** de la app, no por el nombre del ejecutable.

**Pendientes originales** (criterios de §13.2, actualizados arriba):
- **Medir con uso real:** tasa de éxito de `SetForegroundWindow` con más repeticiones (8/8 hasta ahora); latencia del hook con un histograma más fino y con CPU al 100 %; que Alt-antes-de-Caps no abra menús en Explorer, Office, Chrome y VS Code (en el Bloc de notas ya está verificado).
- **Verificar con uso real** el vigía (`Caps` + F12), el modo seguro (`Caps` + F11) y la reinstalación al desbloquear.
- **Documentar ventanas elevadas** (Administrador de tareas al frente) y la variante UIAccess.
- **Prueba de 24 h** y 0 cambios de contexto en reposo (WPR).

### 13.4 Estado del spike 2 (noviembre 2026)

**Implementado** (`wm-platform::hide`):
- **Las cuatro estrategias:** `cloak` (shell, no documentada), `minimize` (con `DWMWA_TRANSITIONS_FORCEDISABLED`), `offscreen` (afuera de la pantalla virtual, sin cambiar el tamaño, con `SWP_ASYNCWINDOWPOS`) y `hide` (`SW_HIDE`).
- **Interfaces del shell** (`IApplicationViewCollection` / `IApplicationView::SetCloak`) declaradas con `windows_core::interface`.
  - Definiciones tomadas de GlazeWM (GPL-3.0, compatible), con atribución en el código.
  - Si Windows no las expone, `Hider::new(cloak)` falla y se puede usar otra estrategia.
- **Marca en la ventana** (`SetPropW`, propiedades `wm.hidden` y `wm.pos`) antes de esconder.
  - `wm restore` / `hide::restore_all` recorre **todas** las ventanas top-level, escondidas incluidas, y muestra las marcadas con el método correcto.
  - No depende de la memoria del proceso que las escondió.
- **Herramientas de prueba:**
  - `wm puppet [n]`: ventanas de colores numeradas en otro proceso;
  - `wm hide-test <método> [rondas] [apps...]`: alterna dos "escritorios" en el orden mostrar → esconder, mide el tiempo y verifica el estado con `DWMWA_CLOAKED`, `IsIconic`, la posición o `IsWindowVisible`.

**Primera verificación automática** (2 títeres, 3 cambios, `cloak`): **1,5–1,7 ms por cambio**, 0 ventanas en mal estado; `DWMWA_CLOAKED` confirma `DWM_CLOAKED_SHELL`. **El cloaking del shell funciona en Windows 11 24H2 (build 26100) sin admin.**

**Prueba real con 10 títeres (15/11/2026):**

| Método | Tiempo por cambio (10 ventanas) | Lo que se vio | Estado verificado |
|---|---|---|---|
| `cloak` | ~5 ms | **Instantáneo, sin animación, sin parpadeo ni fondo visible** | 10/10 cambios ok |
| `minimize` (transiciones desactivadas) | ~1,4 ms | **Igual que `cloak`**: confirma que `DWMWA_TRANSITIONS_FORCEDISABLED` quita la animación de minimizar entre procesos | 10/10 ok |
| `offscreen` | ~0,9 ms | **Bug:** el títere 1 quedó afuera de la pantalla (ver abajo) | 5 cambios con 1 ventana mal |
| `hide` | ~3 ms | Sin parpadeo. En la barra de tareas no se notó nada porque los 10 títeres están agrupados bajo un mismo ícono | 10/10 ok |
| `cloak` con apps reales (2 Bloc de notas + Brave) | ~2 ms | Las apps funcionan normal después | 6/6 ok |

**Bug encontrado y corregido (`offscreen`):**
- La posición original se guardaba empaquetada en una sola propiedad. El títere 1 estaba en (0, 0), así que el valor guardado era 0, y `GetPropW` devuelve 0 cuando la propiedad no existe: el programa creyó que no había posición y dejó la ventana afuera.
- Después, la prueba con `hide` **pisó la marca**, así que `wm restore` la "mostró" pero en su posición fuera de pantalla.
- **Corrección:**
  - la posición se guarda en dos propiedades (`wm.x`, `wm.y`) con un bit de presencia;
  - esconder una ventana ya marcada primero la restaura con su método original;
  - si falta la posición, la ventana se trae a (50, 50) del monitor principal en vez de dejarla afuera.
- **Verificado:** `offscreen` con el títere en (0, 0) da 0 ventanas en mal estado, y mezclar métodos sobre las mismas ventanas funciona.

**Recuperación tras un corte (spike 3, parte 1):**
- con `hide` (Ctrl+C a mitad): `wm restore` recuperó las 5 ventanas ✅;
- con `cloak` (proceso matado a la fuerza): `wm restore` recuperó la ventana cloaked ✅. **La marca sobrevive a la muerte del proceso** porque vive en la ventana.

**Decisión:** `cloak` como método principal y **`minimize` con transiciones desactivadas como respaldo documentado**: visualmente quedaron iguales. `offscreen` y `hide` quedan como opciones de último recurso.

**Pendiente:**
- ~~Guardián~~, ~~reinicio del Explorer~~ y ~~ventana maximizada~~: resueltos en el spike 3 (§13.5).
- Matriz de apps más amplia (Office, Electron, Java, UWP).

**Hito de producto:** con los spikes 0 y 1 ya existe la **capa de apps** del mapa oficial (`Caps` + inicial). Se puede probar con la usuaria de prueba (`keyboard-design.md` §12) antes de construir el resto.

### 13.5 Estado del spike 3 (noviembre 2026)

Hecho y probado de forma automática la noche del 15/11/2026, sin intervención del usuario.

**Implementado:**
- **`wm guardian [comando...]`** (`app/src/guardian.rs`; por defecto vigila `core`):
  - lanza el mismo ejecutable con ese comando y lo espera bloqueado (0 % de CPU);
  - cuando el hijo termina, **siempre** corre `restore_all`;
  - salida limpia (código 0), error de uso (código 2) o parada pedida por el usuario: termina sin relanzar;
  - cualquier otra salida: relanza tras 250 ms, 500 ms y 1 s; **a la 4.ª muerte en menos de 60 s se rinde**, con las ventanas ya restauradas;
  - Ctrl+C, Ctrl+Break y cerrar la consola (`wm-platform::console`): el guardián no muere, marca la parada y espera al hijo. Al cerrar la consola, el manejador retiene a Windows hasta 4 s (el sistema mata a los ~5 s) mientras el hilo principal restaura.
- **Arranque = recuperación** (§9.5): `wm core` corre `restore_all` antes de instalar el hook y al salir limpio. El guardián también lo corre al arrancar.
- **`wm hidden`**: lista las ventanas con nuestra marca (código 1 si hay alguna, útil para scripts).
- **Reconexión con el shell** (`ShellViews` en `hide.rs`): el objeto COM se pide a demanda; si una llamada falla con un error de "el otro proceso ya no existe" (`0x800706BA`, `0x80010108`, etc.), se descarta, se pide uno nuevo y se reintenta una vez.
- **Herramientas de prueba:** `wm hide-hold <método> [cycle]` (un "core de mentira" que esconde los títeres impares y espera, o alterna cada 15 ms), `wm crash-test [método] [n]`, `wm max-test` y `wm ctrl-c <pid> [break | close]`.

**Resultados:**

| Prueba | Resultado |
|---|---|
| **1000 muertes** de `hide-hold cloak cycle` en un momento al azar (0–200 ms, a mitad de cualquier cambio), restaurar y esperar a que todo se vea | **0 ventanas perdidas**, 0 recuperaciones > 500 ms. Hasta verse todo: p50 3,5 ms, p95 4,2 ms, p99 4,9 ms, máx 11,9 ms ✅ |
| Lo mismo con los otros métodos (300 muertes cada uno) | `minimize` p95 20 ms; `offscreen` p95 7 ms; `hide` p95 28 ms. 0 perdidas en todos ✅ |
| Posición tras 100 muertes por método | **0 títeres fuera de su posición original** ✅ |
| Guardián: matar al hijo 4 veces seguidas | Restauró 5/5 ventanas cada vez (~4 ms), relanzó a los 250/500/1000 ms y a la 4.ª se rindió dejando todo visible ✅ |
| Matar guardián + hijo juntos (`taskkill /T /F`) | Quedan 5 ventanas escondidas (caso previsto en §9); `wm core` al arrancar las restauró: "Restauradas 5 ventanas de una sesión anterior" ✅ |
| Hijo que sale limpio (`wm guardian version`) | No relanza ✅ |
| Ctrl+Break en la consola del guardián | El hijo murió con `0xC000013A`, el guardián restauró 5/5 y no relanzó ✅ |
| Cerrar la ventana de la consola | Igual: 5/5 restauradas, sin relanzar, antes de que Windows matara el proceso ✅ |
| Ventana **maximizada** escondida y mostrada, con los 4 métodos, por `show` y por `restore_all` | **8/8: vuelve maximizada y visible** (la duda sobre `SW_SHOWNOACTIVATE` no se confirmó) ✅ |

**Reinicio del Explorer con ventanas cloaked** (`AutoRestartShell = 1`):
- **Las ventanas siguen cloaked** después del reinicio: el nuevo shell no las destapa solo. La marca en la ventana es lo único que permite recuperarlas.
- Un proceso **nuevo** (`wm restore`) las recupera bien con el shell nuevo ✅.
- **Bug encontrado y corregido:** un proceso que ya tenía el objeto COM abierto quedaba **roto para siempre** (`0x800706BA` en cada esconder/mostrar: 110 errores en 22 cambios). Con la reconexión, solo fallan los cambios mientras el shell todavía no existe (`0x80040154`, "Class not registered", ~2 s) y después sigue normal (1 reconexión, 0 errores el resto) ✅.
- `taskkill /F` sobre el Explorer **no** dispara el reinicio automático en esta máquina: hubo que relanzarlo a mano.

**Lecciones para el core real:**
- **Escuchar `TaskbarCreated`** (mensaje registrado que el shell difunde al arrancar) y reconciliar: lo que debería verse y quedó cloaked durante el corte se muestra, y lo que debería estar escondido se vuelve a esconder. Mientras el shell no exista, usar `minimize` como respaldo o encolar.
- **Lanzar apps con el entorno del usuario**, no con el del proceso: al relanzar el Explorer desde una terminal Git Bash heredó su `PATH` y variables (`HOME`, `MSYSTEM`). Lo mismo pasaría con `Caps` + letra que abre una app si `wm` se arrancó desde una terminal. Solución probada: `CreateEnvironmentBlock(token, FALSE)` + `CreateProcessW` con `CREATE_UNICODE_ENVIRONMENT`.
- El orden mostrar → esconder sigue siendo correcto también con caídas: si muere entre medio, lo peor es una ventana marcada pero visible, que `restore_all` deja igual.

**Pendiente:**
- ~~Ctrl+C real en una terminal~~: **verificado a mano por el usuario el 15/11/2026**, junto con matar el hijo, cerrar la consola con la X y `wm guardian` con el core real (matado con `Stop-Process -Force`, código `0xffffffff`: relanzado a los 250 ms y `Caps` + letra siguió funcionando; `Caps` + `Esc` salió limpio sin relanzar). Tras cada relanzamiento, la primera activación vuelve a necesitar la tecla neutra (una vez por proceso, como en el spike 1).
- `catch_unwind` en el bucle del core y volcado de fallos (§9.3 y §9.10).
- Diario en disco como segunda red (§9.2) y "ningún HWND reciclado tocado": hoy la marca vive en la ventana, así que un HWND reciclado no la tiene; falta la prueba explícita.
- Hacer que el guardián sea el modo normal de arranque (`wm` sin argumentos o al iniciar sesión) cuando exista la bandeja.

### 13.6 Estado del spike 4 (noviembre 2026)

Diseño consultado con Fable 5.2; implementado y medido el 15/11/2026.

**Decisión: tres carriles fijos, separados por de qué dependen** (`wm-platform::ops`):

| Carril | Qué corre | Depende de | Plazo |
|---|---|---|---|
| `focus` | traer al frente (`SetForegroundWindow` + tecla neutra) | foco y hooks LL | 300 ms |
| `win` | mover (`SWP_ASYNCWINDOWPOS`), esconder sin cloak, enumerar, abrir programas | nada que bloquee | 50 ms (abrir: 2 s) |
| `shell` | cloak / uncloak y `restore_all`; único hilo STA | Explorer | 250 ms (restaurar todo: 1 s) |

- **Hilos por app: descartados.** La sonda mostró que ninguna llamada nuestra se traba con la app colgada; lo que sí puede trabarse es Explorer, y lo usan todas las apps por igual.
- **El brain es el dueño:**
  - `submit` nunca bloquea;
  - la cola vive en el brain, con un solo pedido en vuelo por carril;
  - los resultados llegan por un canal y un único `WM_OPS` pendiente a la vez;
  - el brain espera con `MsgWaitForMultipleObjectsEx` y timeout = próximo plazo (`INFINITE` si no hay nada en vuelo): **0 despertares en reposo, sin timers**.
- **Plazo vencido:**
  - el hilo se abandona (nunca `TerminateThread`) y se crea otro **sin esperar a que arranque**: esperar costó 5,2 ms con CPU al 100 %;
  - el hilo viejo, al volver, avisa `Late` y termina;
  - si el trabado es `shell` (Explorer), esconder con cloak pasa a `minimize` hasta que vuelva;
  - el COM de un hilo abandonado no se libera, porque soltarlo sería otra llamada a Explorer.
- **El carril se elige por la marca de la ventana:** `GetPropW` no envía mensajes. Mostrar una ventana marcada `cloak` siempre va a `shell`.
- **Prioridades:**
  - proceso `ABOVE_NORMAL` y sin power throttling;
  - brain `HIGHEST`;
  - carriles `ABOVE_NORMAL`.
- **Consola en un hilo `wm-log`:** el brain nunca hace `println!`, que puede trabarse si el usuario selecciona texto en la terminal.
- **`wm core` ya usa este modelo:** `Caps` + letra pide `Scan` a `win`, resuelve en el brain y pide `Focus` a `focus`. La restauración al arrancar y al salir va por `shell`.

**Sonda de llamadas** (`wm hang-puppet` + `wm hang-probe [sleep|busy]`). El títere colgable vive en su propio proceso y se cuelga dentro del wndproc, dormido o con CPU al 100 %. Cada llamada corre en un hilo aparte con 2 s de plazo:

| Llamada | Sana | Colgada < 5 s | Colgada, con fantasma |
|---|---|---|---|
| Título, visibilidad, rectángulo, `DwmGetWindowAttribute`, `IsZoomed` | 0,1–0,2 ms | 0,1–0,4 ms | 0,1–0,2 ms |
| `user_windows` (enumerar + nombre de proceso) | 0,7 ms | 0,5 ms | 0,4–0,6 ms |
| cloak esconder / mostrar (vía Explorer) | 6–10 ms | 2–5 ms | 6–7 ms |
| `minimize` / `offscreen` esconder y mostrar | 0,1–0,2 ms | 0,1–0,4 ms | 0,1–0,4 ms |
| `activate` (`SetForegroundWindow`) | 0,1 ms | 0,1 ms | **51 ms** (100 ms con la tecla neutra): no se bloquea, pero el foco no llega |
| `restore_all` | 3,5 ms | 3,6 ms | 0,7 ms |
| [control] `SendMessageTimeout(SMTO_ABORTIFHUNG)` | 1,7 ms | 100 ms (su timeout) | 0,1 ms |
| [control] `SetWindowPos` **síncrono** | 0,2 ms | **trabada** | **trabada** |

- **Ninguna llamada nuestra se traba**: solo el `SetWindowPos` síncrono, ya prohibido en §4.4.
- `IsHungAppWindow` da TRUE a los **5,0 s**.
- **Ventana fantasma:**
  - aparece a los ~5 s, es de clase `Ghost`, del proceso `dwm.exe`, y tiene título «… (Not Responding)»;
  - la ventana colgada **sigue visible, sin cloak y en nuestra lista**, así que `Caps` + letra la encuentra;
  - el fantasma ahora se filtra **explícitamente por clase**; antes quedaba afuera por casualidad, porque falla `OpenProcess` sobre `dwm`.
- **Trampas corregidas en `user_windows`:**
  - se descartan las ventanas propias antes de leer nada de ellas;
  - el título se lee con `InternalGetWindowText`, que nunca envía `WM_GETTEXT`. `GetWindowTextLengthW` sí lo envía a ventanas del mismo proceso.

**Prueba de carga** (`wm ops-test [s] [focus] [burn]`):
- un brain real con los carriles;
- un hilo que le hace ping cada 1 ms;
- teclas sintéticas cada 40 ms (cambiar de escritorio con cloak, mover 10 ventanas, enumerar y, con `focus`, traer al frente);
- 9 títeres más el colgable;
- tres fases: normal, colgado (con fantasma) y carriles trabados a propósito (400 ms en `win`, 600 ms en `shell`).

| Criterio del spike | Meta | Sin carga | CPU al 100 % (`burn`) |
|---|---|---|---|
| Vuelta del brain | nunca > 5 ms | máx 0,66 ms, 0 > 5 ms | **máx 0,77 ms, 0 > 5 ms** |
| Tecla → pedido emitido | p99 < 10 ms | p99 0,33 ms | **p99 0,17 ms** |
| Operaciones sobre las demás ventanas, con 1 colgada entre 10 | p99 < 50 ms | `win` p99 3,7 ms, `shell` p99 5,4 ms | **`win` p99 3,5 ms, `shell` p99 7,0 ms** |
| Ninguna llamada usada se bloquea | — | ✅ (sonda) | ✅ |

- **Con carriles trabados a propósito:**
  - los pedidos que esperan detrás de la llamada trabada tardan hasta el plazo: `win` máx 50 ms, `shell` máx 170 ms;
  - el p99 de `shell` queda en ~50 ms; es el costo de tolerar las demoras normales de Explorer;
  - 2 hilos abandonados, 2 vueltas tardías, y 10–15 escondidas hechas con `minimize` mientras `shell` estaba trabado.
- **Explorer bajo carga:** con CPU al 100 %, un cloak ocasional tardó **~95 ms** sin estar trabado. Por eso el plazo de `shell` es de 250 ms y no de 20.
- **Traer al frente:**
  - a una ventana sana: 1,6 ms;
  - a la colgada con fantasma: 102 ms, en su carril, sin afectar al resto.
- **Ping al brain:**
  - sin carga, nunca > 1 ms, ni siquiera con cambios de foco;
  - con CPU al 100 % **y** cambios de foco, picos aislados de 6–14 ms con el brain ocioso: su vuelta más larga fue 0,5 ms, así que es el planificador de Windows y no trabajo nuestro;
  - p99 0,37 ms.
- **`wm core` real con `Caps` + letra simulado:** de la tecla a la ventana al frente en **1,9–4,4 ms**, sin hilos abandonados.

**Pendiente:**
- Investigar con WPR los picos de 6–14 ms del ping bajo CPU al 100 % con cambios de foco: ¿boost de primer plano? ¿DPC?
- Probar Explorer suspendido de verdad (no simulado) y `CoCancelCall` para recuperar el hilo `shell` sin abandonarlo.
- Reemplazar la espera de `activate` (hasta 2 × 50 ms en el carril `focus`) por `EVENT_SYSTEM_FOREGROUND`, y mapear el fantasma a su ventana (`HungWindowFromGhostWindow`, no documentada).
- No mandar pedidos repetidos a una ventana "no responde" (guardar el último deseado y aplicarlo al recuperarse), y fusionar pedidos por ventana en el brain (lógica pura en `wm-model`, con proptest).
- `WinRef` con pid/tid para no actuar sobre un HWND reciclado en pedidos retenidos.
- Escuchar `TaskbarCreated` (necesita la ventana top-level del brain) para reconciliar tras reiniciar Explorer (§13.5).
- Abrir programas con el entorno del usuario (`CreateEnvironmentBlock`), no con el de la terminal (§13.5).

### 13.7 Estado del spike 5 (noviembre 2026)

Plan de medición revisado con Fable 5.2; implementado y medido el 15/11/2026, en Windows 11 24H2 con 2 monitores de 1920×1080 a 144 Hz.

**Decisión: Win32 + Windows.UI.Composition, y el texto del mapa como contornos vectoriales, sin dispositivo D3D propio.**

| Camino de texto | Cómo funciona | Primer cuadro (incluye armar el mapa) | Memoria de la demo con el mapa | Elegido |
|---|---|---|---|---|
| **`shape`** | DirectWrite da los contornos de los glifos → geometría D2D en CPU (sin dispositivo) → `CompositionPath` → `ShapeVisual`. Lo rasteriza DWM | **6–14 ms** | **1,7 MB WS privado / 2,3 MB commit** | ✅ |
| `warp` | D2D sobre D3D11 por software en una `CompositionDrawingSurface` | 26–32 ms | 6,1 MB / 10,5 MB | respaldo |
| `d2d` | Lo mismo con D3D11 por hardware | **136–148 ms** ❌ | 12,8 MB / **54,6 MB** | ❌ |

- Las tres se ven prácticamente iguales: se verificó con capturas.
- **Límites de `shape`:**
  - no hay shaping, kerning ni ligaduras (se toman glifo por glifo);
  - no hay ClearType: el suavizado es en escala de grises;
  - no hay emoji a color.

  Para las etiquetas del mapa (una letra y un nombre corto) no se nota. Si un texto lo necesitara, el respaldo es `warp`.
- **Overlays:**
  - una ventana por monitor (`WS_EX_NOREDIRECTIONBITMAP | TOPMOST | TOOLWINDOW | NOACTIVATE | LAYERED | TRANSPARENT`) con `DesktopWindowTarget` y un solo `Compositor`;
  - **la ventana mide 1 px menos de alto que el monitor**: del tamaño exacto, el shell la tomó por una app a pantalla completa (`SHQueryUserNotificationState` = "ocupado") y silenciaría las notificaciones;
  - no roba el foco;
  - los clics la atraviesan (`WindowFromPoint` devuelve lo que está debajo).
- **Animaciones:**
  - las corre DWM (`Vector3KeyFrameAnimation`, por tiempo, lineal);
  - al esconder se detienen, porque una animación `Forever` mantiene a DWM componiendo aunque no se vea;
  - el core llama a `DisableProcessWindowsGhosting`: un hilo colgado nunca se vuelve una ventana fantasma a pantalla completa.
- **Precrear al arrancar el core:**
  - hilo ui, `DispatcherQueue`, `Compositor`, ventanas ocultas y targets: cuestan 0,3 MB privados;
  - el mapa se arma al mostrarlo (6–14 ms, dentro de los 0,4 s de espera de Caps).
- **`wm.exe` no importa DLL de gráficos:**
  - `CoreMessaging`, `d3d11`, `dxgi`, `d2d1` y `dwrite` se resuelven en tiempo de ejecución (`wm-platform/src/dynlib.rs`), y `psapi` se reemplazó por `K32GetProcessMemoryInfo`;
  - así el guardián, el core y la CLI no las cargan, y una función que no exista en Windows 10 no impide arrancar;
  - la CI lo verifica con `dumpbin`.

**Medición** (`wm-bench overlay` y `wm-bench snapshot`, un binario aparte para no cargarle D3D a `wm.exe`):
- **Desktop Duplication por monitor:**
  - por cada cuadro que compone DWM copia una sola fila de píxeles y ubica el cuadrado magenta de la demo;
  - informa "cuadros de DWM por refresco" (el techo que pone DWM) y "refrescos con posición nueva" (nuestra pila);
  - mide la regularidad del paso (Δx), que revela judder aunque el porcentaje dé bien;
  - invalida la corrida con > 1 % de cuadros salteados y se recupera de `DXGI_ERROR_ACCESS_LOST`.
- **Memoria** de la demo y de `dwm.exe` con `NtQuerySystemInformation` (working set privado y commit), sin abrir el proceso: `dwm.exe` no deja abrirse sin elevación.
- La demo (`wm overlay-demo`) obedece órdenes por stdin, así el "precreado" se mide desde la orden `show`, como lo haría el core.

| Criterio | Meta | DISPLAY1 (144 Hz) | DISPLAY2 (144 Hz) |
|---|---|---|---|
| Refrescos con posición nueva | ≥ 97 % | **99,9 %** (DWM compuso 100 %) | **99,9 %** (100 %) |
| Paso por cuadro | parejo | 12,8 ± 0,6 px | 12,8 ± 0,6 px |
| Animación con nuestro proceso **suspendido 10 s** (más que los 5 s del "no responde") | termina | **99,0 %**, llegó al final, 0 fantasmas | **99,0 %**, llegó al final |
| Primer cuadro en frío, **desde antes de lanzar el proceso** (20 arranques) | < 50 ms | **p50 27,3 / p95 30,3 / máx 31,2 ms** | p50 27,5 / máx 31,3 ms |
| Primer cuadro con todo precreado (orden `show`) | — | 3,8 ms (volver a mostrar: 8,4 ms) | 4,0 ms (8,7 ms) |
| Mapa (`shape`), armándolo al mostrar | < 50 ms | p50 11,0 / máx 14,1 ms | p50 5,8 / máx 11,6 ms |

| Memoria | Meta | Medido |
|---|---|---|
| `wm.exe` sin overlays | ≤ 15 MB | **1,0 MB** WS privado / 1,4 MB commit |
| Con overlays y Compositor (ocultos o animando) | ≤ 30 MB | **1,3 MB** / 1,9 MB |
| Con el mapa (`shape`) | ≤ 30 MB | **1,7 MB** / 2,3 MB |
| Δ de `dwm.exe` con el mapa | medir | ~+1 MB, dentro del ruido entre corridas (35–37 MB WS privado en reposo) |

- **CPU de la demo animando:** 0,000 % (0,00 ms en 5 s).
- **Duplicación:** no se recreó en ninguna corrida; 0 cuadros salteados.

**60 + 144 Hz** (corridas del usuario, 15/11/2026, un monitor a 60 Hz por vez):

| Configuración | Monitor de 144 Hz | Monitor de 60 Hz |
|---|---|---|
| Secundario a 60 Hz | 99,9 %, paso 12,8 ± 0,7 px | DWM entregó 720 cuadros en 5 s ("240 %"), todos con posición nueva |
| **Principal** a 60 Hz | **99,9 %**, paso 12,8 ± 0,6 px | ídem |

- **El monitor rápido no se degrada** aunque el principal sea el lento: en Windows 11, DWM compone al ritmo del más rápido, no del principal. Era el riesgo mayor, porque en Windows 10 todo bajaría a 60.
- **En el monitor de 60 Hz, Desktop Duplication entrega cuadros a 144 por segundo:**
  - DWM compone esa salida al ritmo del monitor rápido y la pantalla toma uno de cada ~2,4;
  - por lo tanto **cada refresco a 60 Hz muestra una posición nueva** (criterio ≥ 97 % cumplido);
  - **lo que no se puede medir así es el judder:** si la pantalla toma alternadamente 2 y 3 de esos cuadros, el paso visible sería 25,6 / 38,4 px en lugar de 30,7 px parejos. Desktop Duplication ve lo que compone DWM, no lo que escanea el monitor.
- **El resto, igual que a 144 + 144:**
  - frío p95 24 ms;
  - mapa `shape` 5–16 ms;
  - la animación termina con el proceso suspendido;
  - 0 % de CPU.

**Pendiente:**
- **Judder en el monitor de 60 Hz con otro de 144:** medirlo con `DCompositionGetTargetStatistics` por monitor (Windows 11), o con una filmación en cámara lenta (240 fps) del monitor de 60 Hz. Si hay judder, es de DWM y no lo controlamos, pero conviene saberlo antes de prometer "fluido en todos los monitores".
- **Windows 10 22H2** (una VM alcanza): ahí el criterio razonable es "funciona", porque DWM usa un solo reloj.
- **Control de la medición:** `DCompositionGetTargetStatistics` (Windows 11, por `GetProcAddress`), con y sin duplicación activa, para medir cuánto interfiere el medidor.
- **Juegos a pantalla completa:** que un overlay visible no fuerce composición (PresentMon), y que el overlay se oculte con apps a pantalla completa (§5.7).
- **DPI distinto por monitor:** los visuales van en píxeles físicos; probar con 125–200 %.
- **HDR:** Desktop Duplication convierte a BGRA8 y el umbral de magenta podría fallar.
- **Reinicio de DWM** con overlays creados.

### 13.8 Estado del spike 6 (noviembre 2026)

Diseño revisado con Fable 5.2; implementado y medido el 15/11/2026 a 144 Hz (no hay monitor de 240 Hz).

**Decisión: la animación de ventanas existe**, con fantasma por `DwmRegisterThumbnail` público (`wm-platform::ghost`) y un hilo que la mueve un paso por cuadro del compositor. El ordinal 147 (`DwmpCreateSharedThumbnailVisual`) no hace falta; queda como experimento por si en otra máquina el camino público no llega.

**Implementación:**
- **Anfitrión:**
  - una sola ventana sobre toda la pantalla virtual, con los mismos estilos que los overlays (`NOREDIRECTIONBITMAP | LAYERED | TRANSPARENT | TOPMOST | TOOLWINDOW | NOACTIVATE`) y 1 px menos de alto;
  - no oscurece nada, no toma clics y sirve para cruzar de monitor.
- **La miniatura es el marco visible:**
  - `DwmQueryThumbnailSourceSize` dio exactamente `DWMWA_EXTENDED_FRAME_BOUNDS` (370×509 en el títere, 886×643 en Brave y Terminal);
  - el destino va en ese espacio, 1:1: el canje fantasma ↔ real no salta ni se ve borroso;
  - `SetWindowPos` sigue usando `GetWindowRect`.
- **La miniatura de una ventana con cloak sigue en vivo** (estaba **[A VERIFICAR]** en §7.4).
- **Marcapasos:** `DCompositionWaitForCompositorClock` + `DCompositionBoostCompositorClock` durante el vuelo, resueltos en tiempo de ejecución (`dynlib`); `DwmFlush` en Windows 10. La posición se calcula **para el cuadro siguiente** (tiempo + 1 refresco): sin eso, el primer cuadro de cada vuelo repetía posición (96–97 % en vez de ~100 %).
- **Secuencia** (`wm ghost-demo`):
  1. fantasma en el marco viejo, 2 cuadros;
  2. cloak **confirmado** con `DWMWA_CLOAKED`, 1 cuadro;
  3. **un solo** `SetWindowPos` asíncrono al destino, **que además la sube al frente** (`HWND_TOP`, sin activar);
  4. vuelo;
  5. esperar a que el marco real esté en el destino (tope 150 ms; si no llega, el fantasma va adonde está la real);
  6. uncloak **confirmado**, 2 cuadros;
  7. quitar el fantasma.
- **Hallazgo de diseño:** sin el `HWND_TOP` del paso 3, al terminar la ventana real quedaba **debajo** de otras y "desaparecía", porque el fantasma se ve encima de todo y la real no. Con la ventana ya al frente, no pasa.

**Medición** (`wm-bench ghost`):
- un **telón magenta** del tamaño del monitor justo debajo de la ventana; Desktop Duplication cuenta lo que no es telón en una fila a 3/4 de altura (la sombra sobre el telón cuenta como telón);
- **defecto** = cualquier cuadro, transiciones incluidas, con ese ancho a más de **±3 px** del de reposo: menos es "sin ventana", más es "doble imagen", aunque sea parcial;
- el vuelo se toma de las muestras: del primer cuadro movido al primero en el destino.

| App (ventana nueva, abierta para la prueba) | Vuelo | Vuelos | Refrescos con el fantasma en otra posición (meta ≥ 95 %) | Cuadros sin ventana / con doble imagen (meta 0) |
|---|---|---|---|---|
| Títere (Win32) | 600 px en 200 ms | 20 | 100 % | **0 / 0** |
| Brave (Chromium) | 800 px en 200 ms | 60 | 100 % | **0 / 0** |
| Brave | 800 px en 150 ms | 40 | 100 % | **0 / 0** |
| **Brave con CPU al 100 %** (`wm burn`) | 800 px en 200 ms | 40 | **98,8 %** | **0 / 0** |
| Windows Terminal (XAML) | 800 px en 200 ms | 40 | 100 % | **0 / 0** |
| **Rayo** (winit, Rust nativo; el administrador de archivos del usuario, su ventana real) | 800 px en 200 ms | 40 | 100 % | **0 / 0** |

- En todos los vuelos la ventana real llegó al destino a tiempo.
- Con una ventana del usuario, la demo guarda posición, tamaño y si estaba maximizada, y la deja igual al terminar: Rayo volvió exactamente a su rectángulo.
- **Error de medición encontrado:** la demo avisaba "lista" antes de que se pintara el telón, y el ancho en reposo salía de cuadros sin telón (1920 px). Ahora avisa con el telón pintado, y el reposo se toma de los últimos cuadros antes del primer vuelo.
- Un solo `SetWindowPos` por vuelo.
- 0 ventanas marcadas al terminar.

**Pendiente:**
- **Explorer:** no se pudo probar porque en esta máquina el administrador de archivos predeterminado es **Rayo** (un `rayo.exe` nativo, sin WebView2): `explorer.exe <carpeta>` le pasa la carpeta a Rayo y nunca abre una ventana `CabinetWClass`. No era una avería. Probar con el Explorer clásico requiere otra máquina o abrirlo de otra forma; mientras tanto, Rayo es la app de archivos real del usuario de prueba.
- **Lección para el producto:** el "explorador de archivos" no es siempre `explorer.exe`. Las asignaciones por defecto de `Caps` + letra (`E` = archivos) tienen que respetar el administrador de archivos predeterminado.
- **VS Code / Electron:** no está instalado. Brave cubre el mismo motor (Chromium); falta una app Electron real.
- **240 Hz:** no hay monitor.
- **Cambiar de tamaño (agrandar / achicar):** la miniatura en vivo muestra el repintado tardío de la app. Plan de Fable:
  - recortar 1:1 (`rcSource` del tamaño en curso), sin estirar;
  - al agrandar, `SetWindowPos` al principio; al achicar, al final;
  - medir recorte contra estirado.
- **Hilo `anim` en el core:**
  - prioridad `TIME_CRITICAL` solo durante el vuelo; en esta prueba corrió a prioridad normal y alcanzó igual el 98,8 % bajo carga;
  - el brain tiene que ignorar los `CLOAKED`/`UNCLOAKED` de ventanas en vuelo.
- **Sin animación** con: ventanas maximizadas, minimizadas o a pantalla completa; contenido protegido (la miniatura sale negra); apps colgadas (`IsHungAppWindow` antes de empezar); y cloak degradado (§6.3).
- **Esquinas redondeadas y sombra:** la sombra desaparece durante el vuelo. Mirar las esquinas en capturas; si difieren, `DWMWCP_DONOTROUND` en las ventanas en mosaico.

### 13.9 Estado del spike 7 (noviembre 2026)

Diseño revisado con Fable 5.2. Lo automatizable se implementó y midió el 15/11/2026; las pruebas con cambios físicos de pantallas las hace el usuario con el guion de abajo.

**Decisión: motor de persistencia = una instantánea del layout por topología, y reconciliar *después* de la tormenta.** No se pelea con Windows mientras reacomoda ventanas.

- **Identidad de monitor** (`wm_model::screens::MonitorId`):
  - es la tupla EDID (fabricante + producto) + **número de serie** + conector;
  - la serie sale del EDID en `HKLM\SYSTEM\CurrentControlSet\Enum\DISPLAY\…\Device Parameters\EDID`, bytes 12–15, sin admin;
  - el LUID del adaptador, `HMONITOR` y `\\.\DISPLAYn` no sirven: cambian entre arranques;
  - la ruta del dispositivo sola tampoco: no lleva la serie y cambia si se mueve el cable.
- **Reconocer un monitor:** `match_monitors` empareja por niveles: EDID + serie + conector › EDID + serie (cable movido) › EDID + conector (sin serie: sigue al conector) › EDID solo, si es único en los dos lados.
- **Clave de topología:** el conjunto de identidades. Resolución, escala y rotación no entran.
- **Posiciones** relativas al área de trabajo (`RelRect`): exactas en la misma geometría y proporcionales en otra. Los bordes compartidos se conservan.
- **Nunca fuera de pantalla** (`reachable` / `rescue`):
  - el marco tiene que compartir con algún área de trabajo al menos `min(ancho, 120 DIP)` × 48 DIP, con la barra de título adentro;
  - si no, se mueve lo mínimo (sin cambiar el tamaño);
  - las minimizadas no cuentan.
- **Fin de la tormenta** (`StormTracker`, puro y sin timers):
  - cada aviso congela y pide 400 ms de silencio;
  - después, dos instantáneas de la topología a 250 ms tienen que coincidir;
  - recién entonces se reconcilia, una sola vez;
  - un control más a +3 s, por la ola tardía de Windows;
  - tope de 10 s;
  - nunca con la sesión bloqueada.
- **Mientras reconecta, Windows 11 recuerda posiciones por monitor y mueve ventanas solo.** Se lo deja terminar y después se reconcilia contra la instantánea de esa topología.
- **Pendiente para el core** (cuando existan los workspaces):
  - estado en `%LOCALAPPDATA%\wm\state.json`, local porque los ids son de esta máquina; JSON versionado con escritura atómica; guardado con 2 s de demora y nunca durante una tormenta;
  - ventanas de la sesión por HWND + marcas `SetPropW`; entre reinicios, por (exe, clase, título), solo con `Hyper` + `R`;
  - un monitor que desaparece deja a sus ventanas "de visita" con `home` recordado, y vuelven cuando reaparece.

**Implementado y medido:**
- **`wm displays`:** dos LG UltraGear distinguibles por producto (`5bb1` / `5bb2`), **serie (508158 / 224786)** y conector (`uid4352` HDMI / `uid4353` DisplayPort).
- **Correcciones de robustez:**
  - una topología a medio cambiar (un camino sin su monitor) da **error** en vez de una lista más corta, que haría mover ventanas por un monitor "desaparecido";
  - `QueryDisplayConfig` se reintenta como mucho 4 veces;
  - en espejo, un monitor por fuente.
- **Colocación por marco visible** (`wm-platform::geometry`):
  - el margen (`GetWindowRect` − `EXTENDED_FRAME_BOUNDS`) se mide por ventana en el momento; si la ventana cruza a otro DPI, la primera pasada lo escala;
  - verificar y corregir **una** vez más solo si el error pasa de 1 px;
  - en un cruce de DPI no se da por terminado hasta que la ventana informe el DPI nuevo (`GetDpiForWindow`) y el marco quede quieto; un control a +250 ms mide si la app se movió tarde.

| Prueba (100 %, 2 monitores) | Resultado |
|---|---|
| `place-test`: 10 lugares por monitor (full, mitades, cuartos, tercios) × 2 monitores, en títere, Brave y Windows Terminal | **60/60 con 0 px de error**, 1 pasada, ~8 ms cada una |
| `place-test cross`: monitor 1 ↔ monitor 2 (mismo DPI, línea de base) | 6/6 con 0 px, sin movimientos tardíos |
| Lógica pura (`wm-model::screens`): emparejamiento (2000 casos al azar: biyección parcial, mismo EDID, independiente del orden), rescate (20 000 marcos al azar: siempre alcanzable, idempotente, sin cambiar tamaño), `RelRect` (10 000 ida y vuelta exactas; bordes compartidos), tormenta (termina siempre; espera con sesión bloqueada; una reconciliación por calma) | **33/33 pruebas** |
| Prototipo del motor (`wm display-watch restore`): se movió un títere a mano y se le mandó a la ventana del vigilante un `WM_DISPLAYCHANGE` falso | Calma → **configuración conocida** → «devuelta "Títere 1" a su lugar» → ciclo 1 ok |

- **Detalle de Windows:** `WM_SETTINGCHANGE` no se puede postear (lleva un puntero: `PostMessage` falla), solo enviar; `WM_DISPLAYCHANGE` sí.

**Guion para el usuario** (~15 min), con `wm display-watch restore` corriendo en una terminal:
1. **Escala:** monitor 2 a 150 %, después a 200 %, y de vuelta a 100 %. En cada escala, en otra terminal: `wm place-test app windowsterminal cross 10`, y lo mismo con Brave.
2. **Cable:** desconectar y reconectar cada monitor 5 veces; además, 3 veces apagarlo con el botón.
3. **Bloqueo:** `Win` + `L` y volver, 10 veces.
4. **Reposo:** suspender y despertar 3 veces; una de ellas, desconectando un monitor durante el reposo.

Al final, `Ctrl` + `C` en el vigilante imprime el resumen. Criterio: todos los ciclos "ok", configuraciones reconocidas al volver y 0 ventanas fuera de pantalla sin rescatar.

**Resultado de las pruebas manuales** (usuario, 15/11/2026):

| Prueba | Resultado |
|---|---|
| **Cruce de DPI**, monitor 2 a **150 %** (144 DPI) ↔ 100 %: `place-test cross 10` en Windows Terminal y en Brave | **20/20 con 0 px**; cada cruce, **2 pasadas** (meta ≤ 2), ~67 ms; 0 movimientos tardíos |
| Lo mismo con el monitor 2 a **175 %** (168 DPI) | **20/20 con 0 px**, 2 pasadas, ~67 ms, 0 tardíos |
| **200 %** | No disponible: Windows ofrece hasta 175 % en un monitor 1080p, y la escala personalizada exige cerrar sesión |
| **Cables:** 9 desconexiones y reconexiones (o apagados con el botón) de uno u otro monitor | **18 ciclos ok**. "Solo el 1", "solo el 2" y "los dos" se reconocieron **nuevas la primera vez y conocidas siempre después**, con cables reales. 0 ventanas fuera de pantalla |
| **Bloqueo:** `Win` + `L` × 10 | 10 ciclos ok |
| **Reposo** | No se pudo: la PC del usuario no entra en suspensión |
| **Total de `display-watch restore`** | **31/31 ciclos ok**, 0 rescates y 0 restauraciones necesarias |

- Con cables reales, Windows 11 ya dejó las ventanas en su lugar, así que el motor no tuvo que mover nada. Había solo 2 ventanas abiertas: la restauración en sí está probada con la simulación (un títere movido + `WM_DISPLAYCHANGE` falso), no con cables.
- **Cada cambio de cable** trae una ráfaga de 3–5 avisos en ~0,5 s: `dispositivos` (uno o dos), `pantallas` y `área de trabajo` (una o dos veces). **Cada cambio de escala** trae `configuración`, `pantallas` y `área de trabajo`. La calma de 800 ms + 250 ms los agrupó siempre en un solo ciclo.
- En el monitor de 150–175 %, la primera pasada con el margen escalado no alcanza, porque la app se reescala después; la segunda corrige. Por eso los cruces dan siempre 2 pasadas, y dentro del criterio.

**Sobre el 50/50:** 0 fallas en 31 ciclos acota la tasa real a ~9 %. Para acercarse a 50/50 hace falta repetir las trazas reales en la lógica pura y dejar `display-watch` (sin `restore`, solo observando) varios días de uso normal.

**Pendiente:**
- reposo (en otra máquina, o cuando la del usuario suspenda), más ventanas abiertas al desconectar, y 200 % en un monitor que lo permita;
- la integración en el core: el sumidero de avisos en el brain, `Op::Displays` en el carril `win` y el estado en disco;
- la colocación por lotes en el core: todas las ventanas a la vez, una sola espera y una segunda pasada solo para las que fallaron; el sondeo bloqueante actual sirve para medir, no para el carril (plazo de 50 ms);
- títeres "difíciles": sin DPI awareness, PMv1, sin borde, con tamaño mínimo y que se reescalan solos con demora;
- validar `EXTENDED_FRAME_BOUNDS` contra los píxeles con el telón de `wm-bench`.

### 13.10 Estado del spike 8 (noviembre 2026)

Planteo revisado con Fable 5.2; implementado y medido el 15/11/2026. **Decisión de toolkit: pendiente de confirmación del usuario.** La recomendación está al final.

**Las dos versiones** muestran la misma pantalla, con los mismos datos (`wm_model::settings_demo`: 26 letras, 12 asignadas, 40 apps):
- búsqueda que filtra en vivo;
- perfil con dos botones de opción;
- lista A–Z de selección única y un desplegable que edita la letra elegida;
- tres opciones;
- estado con plural ("12 letras asignadas");
- Guardar, deshabilitado hasta que haya un cambio, y Cancelar, con `Enter` y `Esc`.

| | **Slint** 1.18.1 (`wm-settings.exe`, crate `settings`) | **Nativa** Win32 / comctl32 v6 (`wm settings`, `wm-platform::native_settings`) |
|---|---|---|
| Backend | winit + renderer por software (sin GPU), `accessibility` (AccessKit → UIA), estilo `fluent` | `CreateWindowExW` + `IsDialogMessageW`, ~500 líneas |
| **Arranque** (20 en frío, hasta el primer cuadro con la marca) | **p50 111 ms**, p95 116, máx 128 | p50 141 ms, p95 147, máx 207 (el primero) |
| **Memoria** (1 s después de mostrarse) | 4,9 MB WS privado / 5,8 MB commit | **1,5 MB** / 2,2 MB |
| CPU en reposo (3 s) | 0 | 0 |
| **Tamaño** sin información de depuración (GNU) | **10,55 MB** solo para los ajustes | **0,88 MB todo `wm.exe`** (guardián, core, overlays y ajustes) |
| DLL estáticas | `dwrite`, `uiautomationcore`, `comctl32`, `imm32`, `uxtheme`, `shell32`... → **binario aparte** (dentro de `wm.exe` las cargarían el guardián y el core, y la CI lo prohíbe) | solo `comctl32` (si gana, cargarla a demanda) |
| Compilación | +2,5 min en release (LTO); `rust-version` sube a **1.92** | nada extra |
| Aspecto | **Fluent, sigue el modo oscuro** del sistema | clásico claro; el modo oscuro de los controles exige APIs no documentadas |
| **Narrador** (UIA nativa, `wm-bench uia`) | todos los controles con nombre **solo después de agregar `accessible-label`** a la búsqueda, la lista y el desplegable; los interruptores se anuncian como **"botón"** (de alternancia); la lista expone **solo los ítems visibles** (11 de 26); UIA informa los botones de opción, los interruptores y la lista como "sin foco de teclado", aunque en la práctica reciben el foco | **roles correctos** ("casilla", "opción"), los **26 ítems**, nombres de la etiqueta anterior (hay que crear cada etiqueta justo antes de su control) |
| Manejo por UIA | buscar «brave» → 1 ítem ✅; activar una opción ✅ | igual ✅ |
| `Tab` real | búsqueda → 2 opciones → ítem → desplegable → 3 interruptores → Cancelar, más una parada en la ventana | búsqueda → grupo de opciones (se cambia con flechas) → lista → desplegable → 3 casillas → Cancelar |
| **Traducción** | `@tr()` con plurales; los `.po` se compilan **dentro** del binario; `select_bundled_translation` **solo funciona después de crear la primera ventana** (antes falla en silencio) | los **mismos `.po`**, leídos con `wm_model::i18n` (incluidos con `include_str!`) |
| Español / inglés | ✅ / ✅ | ✅ / ✅ |
| IME, teclas muertas, alto contraste | sin probar (AccessKit/winit manejan IME; alto contraste no encontrado en 1.18) | los da el sistema |

- **Herramientas nuevas:**
  - `wm-bench settings` (arranque, memoria y CPU de las dos);
  - `wm-bench uia <exe> [args]`: vuelca el árbol con la UIA nativa y la maneja como un lector de pantalla, incluido `Tab` real;
  - `wm_model::i18n`: lector de `.po` con plurales, 3 pruebas.
- **Trampa:** la UIA de .NET de PowerShell 5.1 muestra todos los controles Win32 como "Pane", porque no carga sus adaptadores. Para medir lo que ve el Narrador hay que usar la UIA nativa (`IUIAutomation`).
- **Licencia de Slint:** GPL-3.0-only O royalty-free O comercial. Con nuestro GPL-3.0-or-later, el binario combinado queda bajo GPLv3, sin cartel obligatorio.

**Recomendación** (decide el usuario): **Slint, en un binario aparte (`wm-settings.exe`)**, con dos reglas:
1. todo control con `accessible-label`;
2. `wm-bench uia` en la CI: 0 controles sin nombre y `Tab` alcanza todo.

- **Por qué:**
  - arranca más rápido;
  - sigue el modo oscuro y el estilo de Windows 11, algo que `findings.md` pide ("GUI simple y linda");
  - es declarativo, y los ajustes van a crecer (selector de apps, perfiles);
  - su costo en RAM (4,9 MB) es solo mientras está abierto.
- **Lo que se paga:**
  - 10,5 MB más en disco y un segundo ejecutable;
  - accesibilidad algo peor: rol "botón" en los interruptores y una lista virtualizada.
- **La alternativa nativa** es mejor en tamaño, RAM y accesibilidad, y conserva un solo ejecutable, pero se ve como una app de Windows 7 en modo claro.

**Pendiente:**
- la decisión del usuario;
- IME japonés, teclas muertas y alto contraste en Slint;
- la medición oficial de tamaño con MSVC en la CI;
- el IPC con el core.

---

## 14. Decisiones pendientes

1. **Licencia.** ✅ **Decidido: código abierto, GPL-3.0** (noviembre 2026). Identificador SPDX: `GPL-3.0-or-later` (la forma que recomienda la FSF). Slint se usa bajo su licencia GPLv3. Las alternativas que se evaluaron:

   | Licencia | Quién la usa | Slint | Uso en el trabajo | Forks cerrados |
   |---|---|---|---|---|
   | **GPL-3.0** (recomendada) | niri, GlazeWM | Bajo su licencia GPLv3, sin cartel de atribución | Libre, sin trámite | No permitidos: cualquier versión distribuida tiene que publicar su código |
   | **MIT + Apache-2.0** (doble) | AeroSpace, yabai (MIT), la mayoría de los crates de Rust | Bajo su licencia royalty-free, **con atribución visible** ("Made with Slint" en Ajustes → Acerca de) | Libre, sin trámite | Permitidos |

   Las dos:
   - son aprobadas por la OSI, así que **habilitan la firma gratis con SignPath Foundation** (§12.3);
   - evitan la fricción de komorebi en las PCs de trabajo (`findings.md` §12.2);
   - son compatibles con el crate `windows` (MIT/Apache).

   **Recomendación: GPL-3.0.** Protege el proyecto de que alguien lo cierre y lo revenda, es lo que usan los dos gestores más cercanos en espíritu (niri y GlazeWM) y simplifica Slint. Si más adelante se quisiera vender una edición (por ejemplo, para Mac), sigue siendo posible, pero conviene tener desde el principio un acuerdo de contribución (CLA o DCO) por si hace falta relicenciar.
2. **Windows 10:** se propone nivel 2. Salió de soporte general en octubre de 2025 y le faltan `DWMWA_BORDER_COLOR`, el reloj del compositor y las esquinas redondeadas. **Valor por defecto mientras no se diga otra cosa: nivel 2.**
3. **Nivel UIAccess:** si entra en la primera versión o se posterga. Implica un segundo binario, firma e instalador. **Valor por defecto mientras no se diga otra cosa: se posterga hasta después de la primera versión**, pero el spike 1 igual documenta qué pasa con ventanas elevadas.

---

## 15. Fuentes

Recopiladas por Fable 5.2 durante su análisis.

**Microsoft:**
- [LowLevelKeyboardProc](https://learn.microsoft.com/en-us/windows/win32/winmsg/lowlevelkeyboardproc)
- [KBDLLHOOKSTRUCT](https://learn.microsoft.com/en-us/windows/win32/api/winuser/ns-winuser-kbdllhookstruct)
- [DWMWINDOWATTRIBUTE](https://learn.microsoft.com/en-us/windows/win32/api/dwmapi/ne-dwmapi-dwmwindowattribute)
- [Compositor clock](https://learn.microsoft.com/en-us/windows/win32/directcomp/compositor-clock/compositor-clock)
- [DCompositionWaitForCompositorClock](https://learn.microsoft.com/en-us/windows/win32/api/dcomp/nf-dcomp-dcompositionwaitforcompositorclock)
- [SetWindowPos](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-setwindowpos)
- [SetForegroundWindow](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-setforegroundwindow)
- [IsHungAppWindow](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-ishungappwindow)
- [ToUnicodeEx](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-tounicodeex)
- [DwmRegisterThumbnail](https://learn.microsoft.com/en-us/windows/win32/api/dwmapi/nf-dwmapi-dwmregisterthumbnail)
- [DwmFlush](https://learn.microsoft.com/en-us/windows/win32/api/dwmapi/nf-dwmapi-dwmflush)
- [IDCompositionDevice::CreateSurfaceFromHwnd](https://learn.microsoft.com/en-us/windows/win32/api/dcomp/nf-dcomp-idcompositiondevice-createsurfacefromhwnd)
- [Security Considerations for Assistive Technologies (UIAccess)](https://learn.microsoft.com/en-us/windows/win32/winauto/uiauto-securityoverview)
- [IVirtualDesktopManager::MoveWindowToDesktop](https://learn.microsoft.com/en-us/windows/win32/api/shobjidl_core/nf-shobjidl_core-ivirtualdesktopmanager-movewindowtodesktop)
- [Virtual Desktop Switching in Windows 10 (blog del SDK)](https://learn.microsoft.com/en-us/archive/blogs/winsdk/virtual-desktop-switching-in-windows-10)
- [Window Features (ventanas message-only)](https://learn.microsoft.com/en-us/windows/win32/winmsg/window-features)
- [Using the Visual Layer with Win32](https://learn.microsoft.com/en-us/windows/apps/desktop/modernize/ui/using-the-visual-layer-with-win32)
- [Composition animations](https://learn.microsoft.com/en-us/windows/uwp/composition/composition-animation)
- [Code signing options for Windows app developers](https://learn.microsoft.com/en-us/windows/apps/package-and-deploy/code-signing-options)
- [winget: Create your package manifest](https://learn.microsoft.com/en-us/windows/package-manager/package/manifest)

**Terceros:**
- [komorebi: interfaces COM privadas](https://raw.githubusercontent.com/LGUG2Z/komorebi/master/komorebi/src/com/interfaces.rs)
- [GlazeWM PR #792 (cloaking)](https://github.com/glzr-io/glazewm/pull/792), [issue #1358](https://github.com/glzr-io/glazewm/issues/1358), [issue #977](https://github.com/glzr-io/glazewm/issues/977), [issue #860](https://github.com/glzr-io/glazewm/issues/860)
- [spotco/glazewm #10 (llamadas bloqueantes a ventanas ajenas)](https://github.com/spotco/glazewm/issues/10)
- [OpenNiri-Windows PR #60](https://github.com/AdEx-Partners-DE/OpenNiri-Windows/pull/60)
- [Raymond Chen: Virtual desktops are an end-user window management feature](https://devblogs.microsoft.com/oldnewthing/20201123-00/?p=104476)
- [ADeltaX: DWM Thumbnails, but with IDCompositionVisual](https://blog.adeltax.com/dwm-thumbnails-but-with-idcompositionvisual/)
- [Blur Busters: Mixed-Hz multimonitor](https://blurbusters.com/new-windows-update-fixes-mixed-hz-multimonitor/)
- [AutoHotkey: A_MenuMaskKey](https://www.autohotkey.com/docs/v2/lib/A_MenuMaskKey.htm), [FAQ](https://www.autohotkey.com/docs/v2/FAQ.htm)
- [vicinae #2158 (RegisterHotKey y ventanas elevadas)](https://github.com/vicinaehq/vicinae/issues/2158), [Capturecard_Viewer PR #451](https://github.com/Mui-MuiMui/Capturecard_Viewer/pull/451)
- [Rust 1.91.0](https://blog.rust-lang.org/2025/10/30/Rust-1.91.0/)
- [GitHub: runners arm64 para repos públicos](https://github.blog/changelog/2025-08-07-arm64-hosted-runners-for-public-repositories-are-now-generally-available/)
- [windows-rs: releases](https://github.com/microsoft/windows-rs/releases), [issue #4867](https://github.com/microsoft/windows-rs/issues/4867)
- [Slint: LICENSE.md](https://github.com/slint-ui/slint/blob/master/LICENSE.md), [FAQ](https://slint.dev/faqs)
- [Notas sobre GPUI (terceros)](https://github.com/intendednull/buiy/blob/main/docs/prior-art/gpui/README.md)
