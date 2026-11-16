# Plan de integración del core: primera versión para la usuaria

Plan armado con Fable 5.2 el 15/11/2026, después de los spikes 1–9. Objetivo: la versión con la que la usuaria de prueba hace la sesión 1 de `keyboard-design.md` §12. **[V]** = verificado en el código o en las mediciones de `architecture.md`; **[S]** = supuesto, a comprobar. Las decisiones del usuario están en §8.2.

**Alcance, en orden:** capa de ventana → mapa al mantener `Caps` → arranque normal (guardián, bandeja, recarga de `apps.txt`) → escritorios 1–4 → mouse (hito 5). **Fuera de v1:** tiling y zonas (incluidos T, R, Q/E de la pila, el modo zonas y arrastrar a una zona), buscador de comandos, bienvenida (y con ella la sugerencia de letras, el perfil y la importación de atajos), "Hyper + letra libre pregunta", UIAccess, macOS, íconos en el mapa (D7), persistencia por topología (`state.json`), escritorios por monitor (D2), el esquema escalera, el IPC por named pipe y los WinEvents de creación y destrucción. El resto de lo que prometen los documentos y no entra en v1 está en **§10** (auditoría del 16/11/2026: nada documentado queda sin lugar).

---

## 0. Hallazgos que condicionan el plan

| # | Hallazgo | Evidencia |
|---|---|---|
| 1 | El hook solo avisa al brain cuando hay una acción (tecla abajo con Hyper). Caps abajo/arriba y el pulgar no generan aviso: hoy no hay con qué armar el mapa ni saber qué direcciones siguen apretadas | [V] `crates/wm-model/src/keyboard.rs:175-190`, `:164-172`; `crates/wm-platform/src/input.rs:566-567` |
| 2 | `Hyper + Esc` cierra el core | [V] `app/src/core.rs:153` |
| 3 | `wm` sin argumentos corre `doctor`; `wm.exe` es de subsistema consola (solo `wm-settings` es GUI) | [V] `app/src/main.rs:86`; `settings/src/main.rs:16` |
| 4 | `geometry::place_frame` es bloqueante (150 ms + sondeo cada 8 ms hasta 600 ms + 250 ms). `Op::Place` tiene plazo de 50 ms y no compensa bordes invisibles | [V] `crates/wm-platform/src/geometry.rs:138-177`; `ops.rs:101`; `windows_list.rs:153-163` |
| 5 | El spike 6 solo midió vuelos **sin cambio de tamaño**; redimensionar quedó pendiente. Casi todas las acciones de la capa de ventana cambian el tamaño | [V] `architecture.md` §13.8 |
| 6 | `user_windows()` descarta toda ventana con cloak: una app en otro escritorio sería invisible para `Caps + letra` | [V] `windows_list.rs:95-102` |
| 7 | La marca guarda el **método** de esconder (`wm.hidden`, `wm.x`, `wm.y`), no el escritorio | [V] `hide.rs:121-123`, `:300-314` |
| 8 | `Hider::hide` vuelve sin hacer nada si la marca ya está: no sirve para "reafirmar" un cloak que quitó el shell | [V] `hide.rs:160-163` |
| 9 | Overlays y anfitrión del fantasma se dimensionan una sola vez, al crearse | [V] `overlay.rs:83-133`; `ghost.rs:103-150` |
| 10 | El mapa actual es una demo: letras y apps fijas, colores propios (no los tokens), siempre en el monitor principal | [V] `overlay_text.rs:74-76`, `:367-369`; `overlay_demo.rs:68` |
| 11 | `display_watch` usa hilo propio + canal que no despierta al brain; su único consumidor sondea cada 200 ms y no usa `StormTracker` | [V] `display_watch.rs:78-86`, `:117-179`; `app/src/display_test.rs:29`, `:108` |
| 12 | No existen: WinEvents, bandeja, `TaskbarCreated`, `catch_unwind`, instancia única, vigilancia de archivos, lanzar con el entorno del usuario | [V] búsqueda sin resultados; `ops.rs:483-486` |
| 13 | `wm-model` no tiene dependencias (tampoco `proptest`); las propiedades usan un xorshift propio | [V] `crates/wm-model/Cargo.toml`; `keyboard.rs:319-331` |
| 14 | `wm.exe` hoy importa kernel32, user32, gdi32, advapi32, dwmapi, ole32/oleaut32/combase, comctl32, wtsapi32. No importa shell32 ni userenv | [V] `objdump -p target/release/wm.exe` |
| 15 | La sesión 1 incluye "hacer el navegador más ancho" (G/V, nivel 2, fuera del alcance pedido) y empieza por "la bienvenida" (fuera de alcance) | [V] `keyboard-design.md:514-523`, `:275` |

---

## 1. Modelo de estado (`wm-model`)

**Módulos nuevos, puros:** `world.rs` (estado + `step`), `place.rs` (slots, combinación, empuje, centrado, cálculo de márgenes), `desks.rs`, `journal.rs`, `mapview.rs`, `sim.rs` (plataforma falsa, solo en tests).

**API única:**
- `World::step(evento, ahora_ms) -> Vec<Efecto>`
- `World::deadline() -> Option<u64>`: próximo plazo que el brain tiene que vigilar
- `World::reconcile(instantánea) -> Vec<Efecto>`
- `World::map_view() -> MapView`

### 1.1 Estructuras

| Estructura | Contenido | Notas |
|---|---|---|
| `WinKey` | HWND + pid | La plataforma verifica el pid antes de cada orden (HWND reciclado; pendiente de §13.6) |
| `Obs` | marco visible, rect exterior, DPI, estado {Normal, Maximizada, Minimizada}, cloaked, marcas leídas {método, escritorio}, elevada, app, título | Siempre viene de Windows |
| `Monitor` | bounds, área de trabajo, DPI, Hz | Se lee **en cada acción**; el índice vale solo para esa observación |
| `Slot` | monitor + `h` {Izq, Todo, Der} + `v` {Arriba, Todo, Abajo} | Se traduce al `Placement` existente [V] `lib.rs:55-87` |
| `Win` | key, app, `desk`, `want_hidden`, `sticky` (no gestionable), `placed {slot, achieved}`, `center {tamaño original, paso}`, `min_size`, `unresponsive`, `in_flight {destino, pasada, próximo chequeo}` | `achieved` = el rect que quedó de verdad, no el pedido |
| `Combo` | última dirección por eje, con hora y ventana | Para esquinas |
| `Journal` | cola de hasta 32 entradas {ventana, antes: {marco, estado, escritorio}} | Global (D3) |
| `Desks` | activo (1-4), último foco por escritorio, orden reciente | |
| `MapState` | Oculto / Armado {plazo, capa} / Visible {capa} / Suprimido | |

**Estado de "repetir" sin timers:** "la ventana sigue donde la dejamos" significa marco actual == `placed.achieved` (±1 px). Si el usuario o la app la movió, deja de coincidir y la próxima tecla cuenta como primera vez (`keyboard-design.md:503`).

### 1.2 Fuente de verdad

| Dato | Fuente de verdad | El modelo |
|---|---|---|
| Existencia, rect, min/max, foco, orden Z, monitores, áreas de trabajo | **Windows** | Caché, refrescada antes de cada acción |
| Escritorio de cada ventana | **La ventana** (prop nueva `wm.desk`, con bit de presencia como `wm.x` [V] `hide.rs:128`) | Copia; se reescribe si falta |
| "Escondida por nosotros" y método | **La ventana** (`wm.hidden`, ya existe) | Intención + confirmación (`DWMWA_CLOAKED`) |
| Escritorio activo, diario de deshacer, ciclo de C, combinación, asignaciones cargadas | **Modelo** | Se pierde al morir el core; es aceptable |

### 1.3 Reconciliación

- **R1.** Antes de decidir, observar (`Inspect` o `Scan`). Los eventos son pistas (§2.2).
- **R2.** Después de ordenar, verificar con plazo, en dos pasadas como mucho [V] `geometry.rs:27`. Se registra lo logrado.
- **R3.** Ante `TaskbarCreated`, fin de tormenta de pantallas, `UNCLOAKED` inesperado o relanzamiento: `Scan` completo y `reconcile`. Garantiza visible ⇔ `desk == activo`, escondida ⇒ marcada, y marco alcanzable (`reachable`/`rescue` [V] `screens.rs:103-150`).
- **R4.** Nunca en bucle: una corrección por ventana y por causa. Si falla, la ventana pasa a `sticky` y se anota en el log.

### 1.4 Invariantes para las pruebas de propiedades

1. Cada ventana gestionada está en exactamente un escritorio.
2. Nunca se emite esconder sin marcar antes (la plataforma ya lo hace [V] `hide.rs:171-194`).
3. Para cualquier secuencia más una caída en cualquier punto, `restore_all` deja todo visible.
4. deshacer(hacer(x)) = identidad en marco, estado y escritorio.
5. Todo destino queda dentro del área de trabajo; mitades y cuartos comparten bordes (ya probado [V] `lib.rs:99-126`).
6. Como mucho un `Place` en vuelo por ventana; gana el último destino.
7. Mapa visible ⇒ Hyper abajo; toda secuencia termina con el mapa oculto.
8. Las propiedades actuales del teclado siguen valiendo con las señales nuevas, y `HyperDown`/`HyperUp` llegan siempre apareados.

---

## 2. Flujo de eventos

```
hook (wm-input) ──PostThreadMessage──► brain: World.step() ──► efectos
                                        ▲        │
   WinEvents (callback en el brain) ────┤        ├─► Ops: focus | win | shell | anim ──► resultados (WM_OPS)
   ui (bandeja, pantallas, shell) ──────┤        └─► ui (PostMessage a la ventana wm-ui): mapa
   handle de cambios en %APPDATA%\wm ───┘
```

### 2.1 Hilos

| Hilo | Hoy | Cambio |
|---|---|---|
| `wm-input` | existe | Señales nuevas `HyperDown`, `HyperUp`, `ThumbDown`, `ThumbUp`, y máscara de direcciones mantenidas en cada acción. Caben en los bits libres 9-15 de `lParam` [V] `input.rs:388-389`. Nuevo `request_reset()` |
| brain | existe | Dueño de `World`. Recibe WinEvents. Su espera acepta un handle. **Nunca entra en un bucle modal** |
| ops `focus` / `win` / `shell` | existen | Órdenes nuevas (§2.4) |
| ops `anim` | nuevo carril | Dueño del `GhostHost` y de su propio `Hider`; plazo = duración + 600 ms. Reusa abandono y aviso tardío [V] `ops.rs:314-345` |
| `ui` | nuevo | Compositor y overlays (mapa). Ventana top-level oculta `wm-ui`: bandeja y menú, `TaskbarCreated`, `WM_DISPLAYCHANGE`, `WM_SETTINGCHANGE`, `WM_DEVICECHANGE`, sesión, `WM_ENDSESSION` |
| `wm-log` | existe | Además escribe a archivo |

**Reglas de mensajería:**
- **ui → brain:** `PostThreadMessageW`, como el hook [V] `input.rs:364-367`. Vale porque el brain no tiene bucles modales; por eso el menú de la bandeja no puede vivir en el brain.
- **brain → ui:** `PostMessageW` a la **ventana** `wm-ui`, no mensajes de hilo: con el menú de bandeja abierto el hilo está en un bucle modal y los mensajes de hilo se pierden [S].
- **Datos grandes** (contenido del mapa): canal + mensaje de aviso, como ops [V] `ops.rs:379-384`.
- **`input::wait`** [V] `input.rs:423-470` tiene que volver también cuando un callback de WinEvent dejó algo en la cola local o cuando se señala el handle.

### 2.2 WinEvents

Se registran fuera de contexto, saltando el proceso propio, en el brain. La API filtra por rango, pid y tid, no por tipo de objeto, así que un rango ancho despierta al proceso aunque el callback descarte el evento [S: medir con WPR].

| Evento | Para qué | Costo en reposo | v1 |
|---|---|---|---|
| `EVENT_SYSTEM_FOREGROUND` | Adoptar ventanas nuevas, orden reciente, foco por escritorio, saltar al escritorio de una escondida | 0 sin usuario | Sí |
| `EVENT_SYSTEM_DESKTOPSWITCH` | UAC o bloqueo: reiniciar Hyper y ocultar el mapa | 0 | Sí, hito 2 |
| `EVENT_OBJECT_CLOAKED` / `UNCLOAKED` | Reafirmar si el shell destapa lo nuestro | ~0 | Sí, hito 4 |
| `MINIMIZESTART/END`, `MOVESIZESTART/END` | Nada que la observación previa a cada acción no dé | | No |
| `EVENT_OBJECT_CREATE/DESTROY/SHOW/HIDE` | Altas y bajas en tiempo real | Alto: todos los objetos de todas las apps | **No en v1** |
| `EVENT_OBJECT_LOCATIONCHANGE` | Llegada al destino | Solo acotado a un pid | No; v1 usa chequeos con plazo |

**Consecuencia: adopción perezosa.** Una ventana nueva se adopta en el próximo `FOREGROUND` (lo normal: nace con foco) o en el próximo `Scan`. Cambiar de escritorio empieza con un `Scan`, así que una ventana aparecida sin foco queda en el escritorio que se abandona. Una ventana cerrada se limpia en el próximo `Scan`; ordenar sobre un HWND muerto falla sin efectos. Cuando llegue el tiling habrá que registrar `SHOW`/`DESTROY` y medirlo.

### 2.3 Qué despierta al brain

- **En reposo: nada.** La espera es infinita si no hay plazos [V] `input.rs:451-452`.
- **Plazos, solo mientras hay algo en curso:** órdenes en vuelo (ya existe), mapa armado (≤ 400 ms, solo con Caps apretada), chequeos de colocación (≤ 600 ms tras una acción), tormenta de pantallas (`StormTracker::deadline` [V] `screens.rs:252-261`), antirrebote de recarga (100 ms).
- Un toque de Caps cuesta dos mensajes al brain (microsegundos) y **nada en el hilo ui**.

### 2.4 Órdenes nuevas en `ops`

| Orden | Carril | Nota |
|---|---|---|
| `Inspect(objetivo)` → `Obs` + monitores | win | Lecturas que no se traban con apps colgadas [V] §13.6 (0,1-0,4 ms) |
| `Scan` extendido | win | Incluye las ventanas con nuestra marca aunque tengan cloak (hallazgo 6). Las cloaked sin marca (escritorios virtuales nativos, UWP suspendidas) se siguen ignorando |
| `Normalize(id)` | win | Sacar de maximizada o minimizada con `ShowWindowAsync`, sin activar |
| `Maximize(id)` | win | Solo si D1 = nativo |
| `SetDesk(id, n)` | win | `SetPropW` funciona entre procesos y con la ventana colgada |
| `Reassert(id)` | shell | Volver a aplicar el cloak aunque la marca exista (hallazgo 8) |
| `FocusShell` | focus | Foco al escritorio cuando el destino está vacío [S] |
| `Fly{…}` | anim | Secuencia del spike 6 [V] `ghost_test.rs:160-200` |
| `Displays`, `LoadBindings`, `Tray(Add/Remove)` | win, win, shell | `Shell_NotifyIconW` habla con Explorer |
| `Launch` (existe) | win | Pasar a `CreateEnvironmentBlock` (lección de §13.5) |

`Shell_NotifyIconW` y `CreateEnvironmentBlock` se resuelven por `dynlib`: shell32 y userenv hoy no se importan (hallazgo 14) y el guardián no debería cargarlas.

### 2.5 Secuencia tipo: `Hyper + pulgar + A`

1. Hook → acción con `mantenidas` (microsegundos).
2. Brain: `step` → `Inspect(ventana al frente)`.
3. Resultado → `step` calcula slot, rect exterior (marco ± márgenes; la cuenta de `geometry.rs:89-98` se mueve al modelo) y entrada de diario → `Place` o `Fly`.
4. Chequeos con plazo (+16, +50, +120, +250, +600 ms): `Inspect` y comparar. Con error ≤ 1 px termina. Si cruza DPI, espera el DPI nuevo. Si quedó en otro tamaño, registra `achieved` y hace una sola pasada de anclaje. Sin cambios a los 600 ms: `unresponsive`.

Latencia esperada: dos saltos de carril, como `Caps + letra` hoy (1,9-4,4 ms de punta a punta [V] §13.6).

---

## 3. Semántica de la capa de ventana

**Ventana objetivo:** la que está al frente al llegar la tecla, si es gestionable. No son gestionables: las nuestras, las que tienen dueño, las de herramienta o sin título [V] `windows_list.rs:55-104`, las elevadas y las de pantalla completa. Con una no gestionable, la tecla no hace nada.

### 3.1 Acciones

> **Versión 3 (15/11/2026), la vigente: "una tecla, una mitad".** Después de usar la versión 2, el usuario la encontró poco intuitiva: mezclaba una escalera en vertical con saltos en horizontal y no era simétrica (mitad lateral + W daba la mitad de arriba, pero mitad de arriba + A daba una esquina). Se compararon en un simulador dos reglas coherentes y el usuario eligió la **directa** (`place::Scheme::Direct`):
> - **Una tecla = esa mitad**, de todo el alto o el ancho, **estés donde estés**: la misma tecla hace siempre lo mismo.
> - **Repetirla** (la ventana ya está en esa mitad) = al monitor vecino, entrando por la mitad opuesta; sin vecino, nada.
> - **Esquina = dos teclas juntas** (W + A…), en cualquier orden; repetir el acorde sobre su esquina la lleva al monitor vecino conservando arriba/abajo. Con teclas sueltas no hay esquinas.
> - Se mantiene todo lo demás de la versión 2: el gesto con origen (el acorde no depende del orden de los dedos), la espera de 50 ms solo al empujar con una tecla sola, el lugar efectivo, F, C y Z.
>
> **Probado por el usuario (15/11/2026):** ~300 acciones con Brave en dos monitores (mitades, empujar, acordes empujando en las dos direcciones, A + D que se anulan, F, C, Z), todas a 0 px; hook con máximo 70 µs. "Todo funciona bien." Detalle corregido: desde maximizada, C alternaba 60 % y 80 % pero el 60 % se anunciaba como "centrada".
>
> **La escalera queda como opción futura del menú de ajustes** (`place::Scheme::Ladder`, implementada y probada, todavía no se puede elegir): cada tecla mueve un paso en su eje (izquierda ↔ todo ↔ derecha, arriba ↔ todo ↔ abajo) conservando el otro, así las esquinas salen en secuencia (↙ + W = mitad izquierda, + W = ↖). Para quien la prefiera; ver `keyboard-design.md` §8.3.
>
> **Por qué la directa por defecto:** la misma tecla hace siempre lo mismo (memoria muscular; la escalera depende de dónde está la ventana); lo más frecuente (izquierda/derecha) queda a una tecla, mientras que en la escalera pasar de la mitad izquierda a la derecha cuesta dos (pasa por todo el ancho, como `Win + →`); y se explica en una línea del mapa, para la usuaria de prueba.
>
> ---
>
> **Versión 2 (15/11/2026), reemplazada por la 3.** Tras la prueba del usuario con dos monitores, la semántica de W/A/S/D se rehízo con Fable 5.2 para que dependa de **dónde está la ventana** y no de la tecla anterior ni del tiempo. La tabla de abajo, "Primera vez / Repetir", queda como historia: la reemplaza esta.
>
> **Por qué:** el usuario encontró que (1) una esquina se convertía en mitad al moverla de lado; (2) apretar W + A "a la vez" daba resultados distintos según qué dedo llegaba primero; (3) una mitad recién empujada + W iba a la esquina del otro lado. Las tres causas: la esquina por secuencia dependía de la tecla anterior y de un plazo de 1 s, recordaba la *tecla* y no el *lado* donde quedó la ventana, y un acorde eran dos acciones independientes.
>
> **La regla** (`place::step`, `place::chord`; `World::arrange`):
> - **R1, una tecla sola**, desde donde está la ventana, solo en su eje. **A / D** van a ese costado y **conservan la altura** (una esquina se mueve como esquina). **W / S** llevan a la mitad de arriba / abajo **de todo el ancho**, también desde una mitad lateral; una **esquina** que se aleja de su borde pasa a la mitad de su costado (↖ + S = mitad izquierda). *Cambio pedido por el usuario (15/11/2026): la primera versión, como `Win + ↑`, llevaba de una mitad lateral a la esquina, y la mitad de arriba quedaba para F y después W ("un comando extra").* **Contra el borde**, al monitor vecino entrando por el borde opuesto y conservando el otro eje; sin vecino, nada.
> - **R2, teclas juntas** (la segunda baja mientras la primera sigue apretada): nombran los bordes que toca la ventana, **desde donde estaba antes de la primera tecla**, así que el orden de los dedos no importa. Dos opuestas se anulan. Si la ventana ya estaba ahí, empuja (primero de costado, después en vertical).
> - **Sin plazo de 1 s.** "D y, cuando sea, W" da la esquina por estado.
> - **Empujar con una tecla sola espera 50 ms**, por si es la primera de un acorde: no cambia el resultado, evita ver la ventana ir al otro monitor y volver.
> - **Dónde está la ventana** (para la tecla siguiente): adonde va si hay una colocación en curso; donde la dejamos si el marco no cambió (aunque no haya podido achicarse del todo); si no, lo que se ve (mitad, esquina o libre). Maximizada cuenta como libre.
>
> | Desde | A | D | W | S |
> |---|---|---|---|---|
> | Libre o maximizada | Izq | Der | Arr | Aba |
> | Izq | ⇐ Der | Der | Arr | Aba |
> | Der | Izq | ⇒ Izq | Arr | Aba |
> | Arr | ↖ | ↗ | ⇑ Aba | Aba |
> | Aba | ↙ | ↘ | Arr | ⇓ Arr |
> | ↖ | ⇐ ↗ | ↗ | ⇑ ↙ | Izq |
> | ↗ | ↖ | ⇒ ↖ | ⇑ ↘ | Der |
> | ↙ | ⇐ ↘ | ↘ | Izq | ⇓ ↖ |
> | ↘ | ↙ | ⇒ ↙ | Der | ⇓ ↗ |
>
> (⇐ X = al monitor de ese lado, a X; sin vecino, nada.)
>
> **Decisiones del usuario (15/11/2026, por recomendación):** el acorde sobre su propia esquina empuja; las teclas opuestas juntas se anulan (para no sumar comandos); W/S en escalera, como Windows; espera de 50 ms solo al empujar con una tecla sola. **Con el cambio de W/S:** desde cualquier mitad o esquina se llega a todas las demás con teclas solas (prueba `from_any_slot_single_keys_reach_every_other_slot`); de una mitad lateral a una esquina son dos teclas (W y A) o un acorde (W + A).
>
> **Carrera encontrada por la CI (corregida):** soltar Alt (aviso inmediato del hook) podía procesarse entre la respuesta de la primera tecla de un acorde y la de la segunda, y la segunda se tomaba como sola. Ahora si una tecla continúa el gesto se decide al llegar la tecla, en el mismo orden que los cambios de capa; prueba que reproduce ese orden.
>
> **Probado por el usuario (15/11/2026) con dos monitores:** los tres casos funcionan (una esquina se mueve como esquina; W + A da siempre lo mismo; mitad empujada + W = esquina de ese lado). Comentario: se había acostumbrado a que esquina + D, tarde, diera la mitad derecha; ahora siempre da la esquina (de ↖ a la mitad derecha: S y D). "Es cosa de acostumbrarse."
>
> **Pruebas:** la tabla completa como datos; A/D nunca cambian la altura; todo empuje vuelve con la tecla opuesta; desde una ventana libre se llega a las 8 ubicaciones; los tres casos del usuario; y la propiedad **"el orden de los dedos no importa"**: 2 monitores × 9 orígenes × 10 acordes × todas las permutaciones × juntas o separadas × app rápida o lenta, mismo resultado. Con el core real (`ci-probe`): mitades y esquinas en 1,3-3,4 ms; empujar en 58-67 ms (con la espera).


| Tecla (con Hyper + pulgar) | Primera vez | Repetir (sigue donde la dejamos) |
|---|---|---|
| A / D | Mitad izquierda / derecha del monitor donde está la mayor parte de la ventana | **Empuja:** al monitor vecino en esa dirección, entrando por la mitad opuesta. Sin vecino: nada |
| W / S | Mitad superior / inferior | Ídem en vertical |
| Dirección + perpendicular | **Esquina.** La perpendicular cuenta si su tecla sigue apretada o si se apretó hace ≤ 1 s sobre la misma ventana y esta no se movió (`keyboard-design.md:669-673`) | Empuja conservando la otra componente |
| F | Maximizar (D1) | Volver al tamaño anterior |
| C | Centrar en su monitor sin cambiar el tamaño. Si eso no produce un cambio visible, pasa al paso siguiente | 60 % → 80 % → tamaño original → 60 %… del área de trabajo, siempre centrada |
| Z | Deshace la última entrada del diario | Sigue hacia atrás; sin entradas, nada |
| Otras letras | Nada (se anota en el log) | |

- **Las esquinas se ejecutan en el acto:** W mueve a la mitad superior y A la lleva a la esquina, sin demora artificial. La alternativa (esperar 50-80 ms antes de cada mitad) penaliza la acción más usada.
- **Vecino:** función pura `neighbor(monitores, desde, dirección)`: el monitor más cercano en esa dirección con mayor solape en el otro eje.

### 3.2 Qué deshace Z

- **Sí:** mitades, esquinas, empujes, F, C y "llevar a escritorio".
- **No:** ir a un escritorio, cambios de foco, abrir apps, movimientos con el mouse, rescates y reconciliaciones.
- Restaura marco, estado (normal o maximizada) y escritorio de la ventana de esa entrada, aunque el usuario la haya movido después.
- Una esquina hecha por combinación es **una** entrada. Si la ventana ya no existe, salta a la entrada anterior en la misma pulsación. Sin rehacer.

### 3.3 Casos borde

| Caso | Comportamiento |
|---|---|
| Maximizada | A/D/W/S/C: `Normalize` y después `Place`, sin animación (§13.8). El diario guarda "maximizada" y Z la vuelve a maximizar. F la restaura |
| Minimizada | No puede ser objetivo (no tiene el foco). Solo vía Z: se restaura sin activar y se aplica |
| Colgada | La orden asíncrona queda en cola y se aplica sola al despertar. A los 600 ms pasa a `unresponsive`, sin reintentos ni animación. Las demás no se frenan [V] §13.6 |
| Tamaño mínimo o no redimensionable | Se acepta lo logrado. Una sola pasada de anclaje: el borde que toca el borde del área queda alineado y el exceso va hacia adentro. `achieved` es la referencia para "repetir". Se recuerda `min_size` |
| Monitor desconectado | El monitor se lee en el momento. Si la ventana quedó inalcanzable, primero `rescue` y después la acción |
| De administrador | Al frente: el hook no recibe teclas y Caps es Bloq Mayús [V] §13.3. En segundo plano: si no se puede marcar, no se esconde y queda en todos los escritorios [S: confirmar que `SetPropW` falla por UIPI] |
| Se reubica sola | No se pelea. Deja de coincidir con `achieved` y la próxima tecla es "primera vez" |
| Cruce de DPI | Primera pasada con margen escalado, segunda con el real; se espera `GetDpiForWindow` [V] `geometry.rs:146-164`; 20/20 con 0 px a 150 y 175 % (§13.9) |
| Pantalla completa | Nada |
| Acción vieja (> 1 s) | Se descarta [V] `core.rs:32`, `:218-224` |
| Dos teclas seguidas sobre la misma ventana | Un `Place` en vuelo; el segundo destino reemplaza al pendiente |

### 3.4 Animación

- Con fantasma solo si: mismo tamaño de origen y destino, ventana normal, no colgada, cloak disponible y el sistema no pide reducir animaciones.
- **Hoy eso cubre pocos casos:** empujar entre monitores iguales y centrar sin redimensionar. El resto es instantáneo hasta que el mini-spike 6b valide vuelos con cambio de tamaño (D6).
- El brain vigila el vuelo. Si vence el plazo: abandona el hilo, pide `Unhide` y apaga las animaciones por esa sesión. La ventana real está marcada durante el vuelo, así que el guardián la cubre.

---

## 4. Escritorios 1-4

| Tema | Diseño |
|---|---|
| **Ir a N** (`Hyper + N`) | `Scan` (adopta lo nuevo al escritorio actual) → mostrar las de N → foco → esconder las del origen. Mostrar y esconder van por el mismo carril (`shell`), así que el orden está garantizado: nunca se ve el fondo (§6.3). El `Focus` se pide cuando terminó el `Unhide` de la ventana elegida. Costo: ~5 ms cada 10 ventanas [V] §13.4. Si N es el actual, nada |
| **Llevar a N** (`Hyper + pulgar + N`) | `SetDesk`, esconder, foco a la siguiente ventana reciente del escritorio actual. No se sigue a la ventana. Entra al diario |
| **Foco al cambiar** | Último foco de ese escritorio; si no, la de más arriba; si está vacío, `FocusShell`. El foco se mueve antes de esconder el origen (problema conocido del cloak, §6.3) |
| **Ventanas nuevas** | Al escritorio activo, en el momento en que se las ve |
| **Diálogos con dueño** | No se gestionan sueltos. [S] el cloak del dueño los arrastra; verificar en el hito 4.0 |
| **`Caps + letra`** | Resuelve sobre todos los escritorios (el orden Z incluye las escondidas). Si la ventana está en otro, cambia de escritorio y la enfoca en una sola transición |
| **Barra de tareas** | Con cloak todas siguen en la barra (§6.2). Clic en una escondida → `FOREGROUND` de una ventana `want_hidden` → saltar a su escritorio |
| **Alt+Tab** | Misma regla si aparece y se elige. [S] si las cloaked aparecen o no; se mira en el hito 4.0 y el diseño sirve en los dos casos |
| **Minimizadas** | Se esconden y se muestran como cualquiera; siguen minimizadas. En modo degradado (`minimize`), las que ya estaban minimizadas ni se marcan ni se tocan |
| **Método** | Autotest de cloak al arrancar sobre una ventana propia (§6.3). Si falla, `minimize` toda la sesión. Con el carril `shell` trabado, la degradación por pedido ya existe [V] `ops.rs:234-244` |
| **Escritorios virtuales nativos** | El actual es "el mundo". Ante `UNCLOAKED` de una `want_hidden` que no esperábamos: `Reassert`, una vez |
| **Reinicio de Explorer** | Las ventanas siguen cloaked y el COM viejo muere; la reconexión ya existe [V] `hide.rs:232-241`, §13.5. `TaskbarCreated` → volver a poner el ícono + `Scan` + `reconcile`. Mientras el shell no exista (~2 s), los cambios fallan y se reconcilian después |
| **Pánico** | `catch_unwind` en el borde del bucle → `RestoreAll` con plazo → salir con error → el guardián relanza |
| **Muere el core** | El guardián restaura todo en ~4 ms y relanza a los 250 ms [V] `guardian.rs:67`, `:108`; §13.5. `wm.desk` sobrevive porque `show_marked` solo quita `wm.hidden` [V] `hide.rs:316-323`. Según D4, el core nuevo vuelve a esconder lo que no es del escritorio activo (el de la ventana al frente; si no, el 1) |
| **Guardián se rinde** | Todo visible + un aviso simple [S] |
| **Salida limpia** | `RestoreAll` + borrar `wm.desk`. Código 0, sin relanzar [V] `guardian.rs:87-90` |
| **Cambio de monitores** | Los avisos van de la ventana `wm-ui` al brain → `StormTracker` (ya puro, con plazos [V] `screens.rs:205-311`) → al reconciliar: `Displays` + `Inspect` de todas, **incluidas las escondidas** (`SetWindowPos` funciona sobre ventanas con cloak [V] `ghost_test.rs:166-170`) → `rescue` → recrear overlays y fantasma. La persistencia por topología (`state.json`) queda afuera: con cables reales Windows 11 ya devolvió las ventanas solo [V] §13.9 |
| **Menú "Mostrar todas las ventanas"** | `RestoreAll` + todas al escritorio activo (§9.6). Es el botón de pánico de la usuaria |

---

## 5. El mapa

### 5.1 Umbral y uso rápido

- **Umbral: 400 ms** con Hyper apretada y sin tecla de acción. Es la constante que ya separa toque de mantener [V] `keyboard.rs:28`, así que "si viste el mapa, Bloq Mayús no cambia".
- **Para no molestar:** (1) una acción antes de los 400 ms suprime el mapa durante **toda** esa pulsación; (2) el pulgar no es acción: elige la capa, y así se descubre la capa de ventana; (3) en un toque no se construye nada; (4) no roba el foco y los clics lo atraviesan [V] `overlay.rs:89-94`; (5) aparece en un solo monitor.
- **Con el mapa visible** (D7): la tecla ejecuta su acción, se pinta en `accent` y el mapa sigue hasta soltar Hyper (`keyboard-design.md:451`).

### 5.2 Ciclo de vida

| Momento | Qué pasa | Costo |
|---|---|---|
| Arranque del core | Hilo ui, `DispatcherQueue`, `Compositor`, una ventana oculta por monitor | +0,3 MB [V] §13.7 |
| `HyperDown` | El brain anota el plazo | 0 |
| Plazo | `Scan` → `map_view` → ui arma lo dinámico y muestra | `Scan` 0,4-0,7 ms; armar 6-14 ms; mostrar 3,8-8,4 ms [V] §13.6, §13.7 |
| Pulgar abajo / arriba | Cambia de capa: dos subárboles, opacidad animada por el compositor | [S] |
| `HyperUp`, Esc, reinicio del hook, `HookStalled`, bloqueo, `DESKTOPSWITCH` o tope de 30 s | Detener animaciones + ocultar | Volver a mostrar: 8,4 ms [V] |
| Cambio de monitores, tras la calma | Recrear `Overlays` | [S] |

- **Precrear lo barato, armar al vencer el plazo.** Armar en `HyperDown` gastaría 6-14 ms del hilo ui en cada toque de Bloq Mayús, para ganar ~10 ms que no se perciben.
- **Caché:** teclas y letras se arman una vez; los nombres se rearman solo si cambiaron las asignaciones, las apps abiertas o el escritorio.
- **Memoria con el mapa:** 1,7 MB de working set privado [V] §13.7, contra una meta de 30 MB.
- **`Hyper + Esc`** pasa a "cancelar y ocultar"; salir queda en la bandeja. Hasta el hito 3 se conserva bajo `WM_TEST=1`.
- **Nunca pegado:** el brain oculta el mapa ante cualquier reinicio de la máquina de teclado, además del tope de 30 s.

### 5.3 Qué muestra

`MapView` es puro, con pruebas de instantánea.

- **Capa de apps:**
  - Fila 1-4: escritorios. El actual en `accent`; los demás con su cantidad de ventanas; vacío en `text-3`.
  - 26 letras en la disposición actual, cada una con la tecla de `design.md` y el nombre de la app: asignada y abierta en `text`; asignada y cerrada, o automática por inicial, en `text-2`; libre, solo la letra en `text-3`.
  - Nombre visible: `settings_demo::APP_NAMES`; si no está, el ejecutable capitalizado.
- **Capa de ventana** (pulgar): W/A/S/D con flecha y palabra, F, C, Z, y 1-4 "llevar". El resto atenuado.
- **Pie:** "letra = app · Alt = acomodar la ventana · números = escritorios" (`keyboard-design.md:453`).
- **Estilo:** tokens oscuros de `design.md` §3.2 (hoy los colores están fijos en `overlay_text.rs:367-369`), Geist por el camino `shape`.
- **v1 es solo texto.** Los íconos piden superficies de mapa de bits: el camino `warp` cuesta 26-32 ms y 6-10 MB [V] §13.7.
- **A comprobar:** que Geist tenga las flechas; si no, se dibujan como formas [S].

### 5.4 Multi-monitor

- **Monitor:** el de la ventana al frente; si no hay, el del cursor; si no, el principal.
- **Tamaño:** hoy 1068 × 312 px fijos [V] `overlay_text.rs:77-84`. Escalar por DPI/96 y limitar al 90 % del ancho [S].

---

## 6. Arranque, bandeja e IPC con los ajustes

| Pieza | Diseño |
|---|---|
| `wm` sin argumentos | `guardian::run(&[])`, que ya lanza `core` por defecto [V] `guardian.rs:32-33`. Cambio en `main.rs:86`; `doctor` queda como subcomando |
| Sin consola | D5: segundo target con el mismo código y `windows_subsystem = "windows"`, que es el que se lanza al inicio. `wm.exe` de consola queda igual para CLI y CI |
| Instancia única | Mutex con nombre en el guardián. La segunda instancia pide "abrir ajustes" a la primera y sale [S] |
| Inicio automático | Clave `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` (sin admin, architecture.md §12.4) con el target sin consola; se activa desde los ajustes o el menú de la bandeja. Sin esto, en las 1–2 semanas de uso real deja de funcionar después de cada reinicio (auditoría §10) |
| Log | `wm-log` también escribe en `%LOCALAPPDATA%\wm\wm.log`, con rotación por tamaño |
| Ícono | Creado en ejecución desde bytes embebidos (`CreateIconFromResourceEx`, user32). Monocromo (`design.md`) |
| Bandeja | La ventana `wm-ui` recibe los clics. `Shell_NotifyIconW` va por el carril `shell`. Menú nativo: Abrir ajustes · Mostrar todas las ventanas (hito 4) · Salir. Clic izquierdo = ajustes |
| Abrir ajustes | `wm-settings.exe` junto al ejecutable. Si el proceso que lanzamos sigue vivo, se trae su ventana al frente en vez de abrir otra |
| Salir | ui → brain `WM_QUIT` → camino limpio existente [V] `core.rs:168-178` → código 0 → el guardián no relanza |
| Apagado o cierre de sesión | `WM_ENDSESSION` en `wm-ui` → restaurar |

### 6.1 Recarga de `apps.txt`

| Mecanismo | Reposo | Edición a mano | Cambios en `wm-settings` | Complejidad |
|---|---|---|---|---|
| **Vigilar el directorio** (`FindFirstChangeNotificationW`, handle en la espera del brain) | 0 % (espera de kernel) | **Sí** | **Ninguno** | Baja |
| Mensaje registrado | 0 % | No | Win32 en un crate que hoy no depende de `windows` | Muy baja |
| Evento con nombre | 0 % | No | Ídem | Baja |
| Named pipe (§4.1) | 0 % | No | Cliente + protocolo | Alta |

**Elegido: vigilar el directorio.** Cubre "Guardar" y la edición a mano. Se vigila `%APPDATA%\wm` (se crea si falta) por `LAST_WRITE | FILE_NAME` (el guardado es temporal + renombrar), con antirrebote de 100 ms; `Op::LoadBindings` en el carril `win` (el brain no hace E/S). El pipe queda para cuando los ajustes necesiten datos vivos del core.

---

## 7. Orden de implementación

**Avance:**
- **Hito 0 (15/11/2026): hecho, salvo `Inspect`, que pasa al hito 1a** (es su primer consumidor). Señales de capa y direcciones mantenidas en el teclado; `wm_model::world` con la capa de apps y una plataforma de mentira en sus pruebas; `app/src/brain.rs` y `core.rs` como cáscara; `WinKey` (HWND + proceso, verificado antes de traer al frente); `Scan` que incluye nuestras ventanas escondidas; log en `%LOCALAPPDATA%\wm\wm.log`; pánico → mostrar todo y salir con 3 (probado con `Caps + F10` en `WM_TEST=1`). En `wm ci-probe`, el chequeo "wm core" maneja el core real con teclas inyectadas.
- **Hito 1a (15/11/2026): hecho en el modelo y con títeres.** `wm_model::place` (direcciones, esquinas, monitor de la ventana, rect exterior con bordes invisibles) y la colocación en `World` (Inspect → Place → chequeos a +16/50/120/250/600 ms → segunda pasada solo si la ventana ya se movió y quedó quieta en otro lugar; la última tecla gana; sin plazos pendientes, el brain duerme). 13 pruebas con una plataforma simulada (app lenta, tamaño mínimo, maximizada, pantalla completa, ventana que desaparece). Con el core real y un títere (`ci-probe`): A → mitad en 2,8 ms, W enseguida → esquina en 1,5 ms, D + S juntas → esquina en 1,7 ms, a ≤ 1 px. **Prueba manual del usuario (15/11/2026), con Brave:** ~200 acciones, todas a 0 px; esquinas encadenadas (W → A → D → S…) pasan de esquina en esquina; 2 necesitaron segunda pasada (91 y 99 ms, probablemente desde maximizada: los bordes invisibles de una maximizada difieren de los de una normal); hook con 658 teclas, p99.9 ≤ 60 µs, máximo 54 µs. Los "ms desde la tecla" del log son hasta la confirmación (primer chequeo a +16 ms), no hasta que la ventana llega (2-7 ms según la sonda). **Pendiente, sin bloquear:** Terminal y Rayo (marcos propios), el otro monitor, y acertar a la primera desde maximizada.
- **Hito 1d (16/11/2026): hecho.** Títeres difíciles (`wm puppet-hard minsize|fixed|slow|relocate|borderless|unaware`, `wm_platform::puppet::HardKind`) y chequeos en `ci-probe` con el core real. **Anclaje** (`place::anchor`): una ventana que no toma el tamaño pedido (mínimo o fijo) queda con el que logró, pegada al borde exterior del lugar pedido (una mitad derecha que no se achica ya no se sale de la pantalla). Desde la segunda pasada, "quieta" alcanza para decidir (antes esperaba hasta el último chequeo: el anclaje bajó de ~670 a ~120 ms). **Colgada:** si a los 600 ms la ventana ni se movió, la app no responde: un solo pedido, sin reintentos ni anclaje (el pedido en cola se aplica al despertar; un anclaje con el tamaño viejo lo pisaba). Medido: minsize 129 ms, fixed 118 ms, slow 122 ms (lo que tarda la app), relocate sin pelearse, borderless y unaware 2,5 ms; con una app congelada 2,5 s, `Caps + W` cambia de ventana en 2,3 ms y la colgada llega a su lugar al despertar. 108 pruebas.
  - **Prueba del usuario (16/11/2026):** minsize, fixed, relocate y borderless bien; Steam (tamaño mínimo 1010 × 600) quedó pegado al borde en las mitades, esquinas y al empujar al otro monitor. Hook máx. 78 µs. Un error: con `fixed`, un acorde cuya esquina (pegada al borde) era justo donde la ventana ya estaba (p. ej. `W` la subió y `W + A` pedía arriba a la izquierda) decía "la app no responde": no se movía y a los 600 ms se la daba por colgada. Ahora, si no se movió pero ya está pegada al borde del lugar pedido, cuenta como "ya estaba ahí". 109 pruebas.
- **Hito 1e (16/11/2026): vuelos del mismo tamaño hechos; falta la prueba del usuario y el mini-spike 6b.** Diseño de Fable 5.2.
  - **Cuándo se anima** (lo decide `World::go`, puro): destino de exactamente el mismo tamaño (1 px de diferencia estiraría la miniatura), misma escala, ventana normal y al frente, no colgada, sin contenido protegido, sin "reducir animaciones" y a 64 px o más. Cubre mitad ↔ mitad opuesta, esquina ↔ esquina, empujar entre monitores con la misma escala, C sin cambiar tamaño y Z de vuelta. Todo lo demás sigue instantáneo.
  - **Efectos:** `Fly` en vez de `Place`; `Land` baja un vuelo antes de cualquier otro pedido sobre esa ventana (acorde que cambia el tamaño, F, Z a maximizada); `Flown` arranca los chequeos de siempre (si la real no llegó, siguen las pasadas de 1d). Una tecla a mitad de vuelo cuenta desde el destino; si es del mismo tamaño, el carril **redirige** el fantasma desde donde va, sin repetir el cloak.
  - **Carril `anim`** (`wm_platform::anim`): un hilo creado con la primera capa de ventana, dormido en `GetMessageW` (0 % de CPU), `TIME_CRITICAL` + reloj del compositor acelerado solo durante el vuelo. Topes en tiempo (cloak 100 ms, llegada 150 ms). Duración `100 + distancia/16` ms entre 110 y 180 (mitad a mitad en 1920: 160 ms). Curva: ease-out cúbico con 25 % lineal (el cúbico puro repetía posición en los últimos cuadros: 94,7 % de cuadros distintos).
  - **Seguridad:** vigilante en el brain (vuelo + 800 ms): abandona el hilo, esconde el anfitrión desde su hilo, muestra la ventana por el carril `shell` y apaga la animación por la sesión. La marca de `Hider` cubre una caída: `restore_all` la devuelve. `WM_ANIM=0` la apaga.
  - **Medido** (144 Hz, títere, `wm-bench ghost puppet … lane`): 600 px en 200 ms 97,8 % y 960 px en 160 ms 96,2 % de cuadros distintos, **0 defectos**. Con el core (`ci-probe`): cloak a los ~10 ms de la tecla y visible en el destino a los ~189 ms; redirigido a mitad de vuelo bien; core matado en pleno vuelo → `restore_all` la devuelve. 123 pruebas del modelo, probe 17/17.
  - **Prueba del usuario (16/11/2026):** mitad ↔ mitad y empujar al otro monitor se animan bien. Lo que cambia de tamaño (mitad → mitad de arriba, esquina → mitad, mitad → esquina) sale instantáneo, como estaba previsto hasta 6b. **Error:** esquina → esquina con dos teclas casi nunca se animaba ("hay que tener un timing perfecto"): la primera tecla llegaba sola y mandaba la ventana a una mitad (otro tamaño), y el acorde ya no era del mismo tamaño. Arreglo: desde una esquina, una tecla sola espera 50 ms por una segunda (como los empujes); desde una mitad no se espera. Pruebas con las teclas a 20 ms y chequeo en `ci-probe` con `W` y `D` a 25 ms (vuela, visible a los ~186 ms). 126 pruebas.
  - **Esquinas probadas por el usuario:** ahora se animan.
  - **Mini-spike 6b hecho (architecture.md §13.8b):** recortar en dos pasos no tiene defectos con el títere, Brave, Rayo ni Terminal (Terminal 90–93 % de refrescos distintos, el resto ≥ 95 %). **Decidido con el usuario: se animan también los cambios de tamaño.** Las ventanas que no toman el tamaño pedido cambian de tamaño sin animación desde la segunda vez. 127 pruebas, probe 17/17.
  - **Prueba del usuario con los cambios de tamaño (16/11/2026):** todo se anima bien en Brave, Rayo y Terminal; Steam la primera vez vuela y queda pegada al borde, después cambia de tamaño sin animación (como se diseñó). Hook máx. 163 µs. Queda ~1 de cada 10 acordes de esquina que pasa por la mitad: la segunda tecla llegaba después de los 50 ms de espera. Ahora se espera 80 ms, y un acorde que llega tarde lo dice en el log ("la segunda tecla llegó N ms después") para afinar con datos. 129 pruebas.
- **Hito 1f (16/11/2026): hecho; falta la prueba del usuario.** `Caps + Alt + G` / `V` agrandan o achican un 10 % del área de trabajo (mínimo 20 %, máximo el área), pegada al borde exterior de donde está: mitad izquierda o derecha → el ancho; de arriba o de abajo → el alto; esquina → los dos, contra la esquina; libre → los dos, alrededor de su centro (corrida para que entre). Repetir sigue con la misma ancla aunque ya no sea una mitad exacta; `A`/`D`/`W`/`S` la devuelven a la mitad exacta (no empujan) y Z deshace cada paso. Maximizada: V → 90 % centrada; G no hace nada. En el máximo o el mínimo, el log lo dice. Anima como cualquier cambio de tamaño. `ci-probe`: "G y V" (mitad → 60 % → mitad, ~138 ms cada una), 18/18. 138 pruebas.
  - **Prueba del usuario (16/11/2026):** G/V funcionan. Error: después de V hasta el límite (Brave no baja de 502 px, ~26 %), ningún cambio de tamaño se animaba más (centrar, G, A, W + A, esquina → mitad): la ventana quedaba marcada entera como "no toma el tamaño". Ahora se recuerda **qué tamaño** no aceptó (mínimo y máximo por eje) y solo va sin animación lo que cae fuera de eso. 139 pruebas.
  - **Segunda prueba (16/11/2026):** todo se anima, también Steam y Terminal. Del log salieron dos detalles, ya arreglados: (1) en el tamaño mínimo de la app, cada V de más volvía a pedirlo (~150 ms para terminar donde mismo): ahora G/V no piden lo que la app ya mostró que no acepta, y el log dice "la app no se achica más"; (2) después de quedar pegada a su mínimo, el V siguiente cambiaba de eje ("20 % de ancho" → "20 % × 90 %"): la ventana anclada ahora cuenta como donde la dejó la tecla anterior. 140 pruebas.
  - **Tercera prueba (16/11/2026):** ese arreglo (2) era demasiado amplio: cualquier ventana pegada al mismo borde contaba como "la misma de antes". Esquina → G hasta ~90 % → D (mitad derecha) → V achicaba hacia la esquina ("40 % × 90 %") en vez de solo el ancho. Ahora solo cuenta si el tamaño es justo el límite que la app no acepta. "Ya está en el tamaño mínimo/máximo" dice el tamaño, para diagnosticar una línea que apareció dos veces sin explicación (después del 30 %). 141 pruebas.
  - **Cuarta prueba (16/11/2026):** el mismo error con Steam (no baja de 1010 px): su mitad derecha queda de 1010 px, justo su límite, y volvía a pasar por "la misma de antes". **Arreglo de raíz:** el ancla de G/V se recuerda solo mientras el último movimiento de esa ventana fue G o V; cualquier otro (A/D/W/S, esquina, C, Z, F) la olvida, en vez de deducirlo por la posición. La línea sin explicar: un V apretado durante el vuelo del V anterior al 20 % cuenta desde ese destino ("ya está en el tamaño mínimo, 384×1032"); es correcto. 142 pruebas.
  - **Sexta prueba (16/11/2026):** Terminal en una franja del 40 % pegada a la derecha → C → Z (vuelve a la franja) → V achicaba los dos ejes: sin ancla de G/V, se decidía por "mitad o esquina exacta" y la franja contaba como libre. **Ahora se decide por los bordes del área que toca:** alto completo y un costado → solo el ancho, pegada a ese costado; ancho completo y arriba o abajo → solo el alto; un borde por eje → los dos, contra esa esquina; si no, los dos alrededor del centro. 163 pruebas. Probado por el usuario con Terminal y Brave (mitades, franjas, C → Z → V, mínimo de la app): siempre solo el ancho.
  - **Quinta prueba (16/11/2026): bien.** Brave y Steam: después de D, V achica solo el ancho (o dice "la app no se achica más" si ya está en su mínimo); después de una esquina, los dos ejes. Hito 1f cerrado. Un pico aislado del hook: máx. 751 µs (p99.9 760 µs con 646 eventos, o sea un solo evento) contra 91–183 µs en las pruebas anteriores; dentro del tope de 5 ms, a vigilar.
- **Hito 2a (16/11/2026): hecho; falta la prueba del usuario.** Diseño de Fable 5.2.
  - **Cuándo** (`wm_model::map`, puro, dentro de `World`): Hyper abajo sin acción 400 ms → `ShowMap` con la capa actual; una acción antes lo suprime toda la pulsación; el pulgar cambia la capa antes o después de mostrarse; con el mapa visible una acción no lo cierra (D7) y Esc lo cierra sin salir del programa (sin mapa, `Caps + Esc` sigue saliendo hasta el hito 3). Se oculta al soltar Hyper, con cualquier reinicio del teclado y a los 30 s (entonces también reinicia el hook). Propiedad con 10 000 secuencias al azar y el `HyperKeyboard` real: siempre termina oculto, solo se ve con Hyper abajo, nunca junto con un cambio de Bloq Mayús.
  - **Bloq Mayús:** el toque ahora dura menos de 350 ms (antes 400, el mismo umbral del mapa): se miden con relojes distintos y una pulsación de ~400 ms podía mostrar el mapa y cambiar Bloq Mayús. Entre 350 y 400 ms no pasa nada.
  - **Reinicios silenciosos** que ahora avisan (`InputEvent::KeyboardReset`): bloqueo de sesión, cambio de escritorio de entrada (UAC, `Ctrl+Alt+Supr`, `EVENT_SYSTEM_DESKTOPSWITCH` en el hilo de entrada), entrar y salir del modo seguro, y el pedido del brain al vencer el tope.
  - **Contenido** (`wm_model::mapview`): las tres filas QWERTY; capa de apps con las asignaciones reales de `apps.txt` (las libres, tenues); capa de ventana con lo que hace cada tecla (W/A/S/D/F/C/Z/G/V).
  - **Hilo `ui`** (`wm_platform::ui`): se crea al arrancar, arma Geist y las formas la primera vez que se muestra en cada monitor, muestra solo la ventana del monitor del foco (o del puntero), vigilante de 500 ms en el brain (si se traba, esconde las ventanas con cloak desde el brain y apaga el mapa), y se oculta solo a los 32 s si el brain se traba. `WM_MAP=0` lo apaga.
  - **Medido** (`ci-probe` "mapa", core real, teclas inyectadas): visible a los ~422 ms de apretar Caps, oculto ~1 ms después de soltar, Bloq Mayús intacto; un toque y `Caps + Q` rápido no lo muestran; visible + acción sigue visible. 19/19. 156 pruebas.
  - **Prueba del usuario (16/11/2026):** el mapa aparece en el monitor del foco (1–5 ms desde el plazo), cambia de capa con Alt, sobrevive a un bloqueo de sesión sin quedar pegado. Error: con Caps mantenida más de 30 s acomodando ventanas (mapa visible, D7), el tope reinició el teclado en pleno uso. Ahora el tope cuenta **sin actividad**: cada acción o cambio de capa lo renueva. 157 pruebas.
- **Hito 2b (16/11/2026): hecho, salvo la fila de escritorios (hito 4); falta la prueba del usuario.**
  - **Contenido:** al vencer el plazo se piden las ventanas abiertas (`Scan`, ~2 ms) y `ShowMap` sale con las dos capas: asignada y abierta en `text`; asignada y cerrada, o automática por inicial (como resuelve `Caps` + letra), en `text-2`; libre, solo la letra en `text-3`. Nombre visible (`APP_NAMES`, alias como `steamwebhelper` → Steam, o el ejecutable con mayúscula). Si se suelta Caps antes de la respuesta, no se muestra nada.
  - **Tecla en `accent`:** con el mapa visible, la letra recién usada se pinta en `accent` (`Effect::MapKey`); se despinta en la pulsación siguiente.
  - **Estilo:** tokens oscuros de design.md §3.2 (`surface`, `border`, `text`, `text-2`, `text-3`, `accent`), panel `bg` casi opaco con radio 14, pie en `text-2` ("letra = ir a la app · Alt = acomodar la ventana"; en la capa de ventana, "dos juntas = esquina · repetir = otro monitor · Z = deshacer"). Etiquetas que no entran se achican.
  - Revisado en capturas (solo del rectángulo del mapa, no compartidas). `ci-probe` 19/19; 161 pruebas.
  - **Prueba del usuario (16/11/2026):** funciona. Dos detalles, arreglados: (1) el mapa desaparecía en pleno uso a los ~32 s: el autocierre de respaldo del hilo `ui` contaba desde que apareció; ahora cada pedido lo renueva; (2) la tecla usada quedaba en `accent` y parecía trabada: ahora destella (firme ~160 ms y vuelve a `surface` en 450 ms, lo anima DWM). Además, el fantasma de un vuelo tapaba el mapa (los dos están encima de todo): ahora el carril `anim` lo pone justo debajo del mapa visible.
- **Hito 2c (16/11/2026): hecho; falta la prueba del usuario.**
  - **Fundido** de entrada de 120 ms (opacidad, lo anima DWM); instantáneo con "reducir animaciones". Ocultar sigue siendo instantáneo: nunca queda a medio camino.
  - **Escala por DPI:** el mapa se dibuja a la escala de su monitor (medidas a 96 DPI × DPI/96), limitado al 90 % del monitor. No probado a otra escala en esta PC (las dos están al 100 %; no cambio la configuración de pantalla).
  - **Monitores:** al mostrarse, si los monitores cambiaron (cantidad, rectángulos o escala), se rearman las ventanas con el mismo compositor (`Overlays::rebuild_screens`) y el contenido.
  - **Pantalla completa:** si la ventana al frente cubre todo su monitor (juego, video), el mapa no se muestra ("Mapa de teclado no mostrado: …" en el log). `ci-probe`: títere sin borde a pantalla completa → sin mapa.
  - **Flechas** en W/A/S/D (Geist tiene U+2190–2193, verificado en su tabla `cmap`).
  - **Medido:** 12,8 MB de working set sin mapa, 21,8 MB después de mostrarlo (meta ≤ 30 MB); 3,3 MB privados; **0,0 ms de CPU en 10 s** con el mapa oculto. `ci-probe` 19/19.
  - **Prueba del usuario (16/11/2026):** el mapa no aparece sobre un video de Brave a pantalla completa, y A/W/S/D no lo tocan. Hueco: C (y F, Z) sí intentaban moverlo y terminaba en "la app no responde" (la app ignora el pedido). Ahora C, F y Z también respetan la pantalla completa (Z deja la entrada para después). 164 pruebas.
- **Hito 3 (16/11/2026): hecho; falta la prueba del usuario.** Diseño de Fable 5.2, con **dos desvíos** del plan: la bandeja vive en un hilo propio (`wm-tray`, no en el del compositor ni en el carril `shell`: el menú es modal, Explorer colgado lo frenaría, y "Salir" no puede depender del mapa) y la recarga también (`wm-watch`, no en un carril de `ops`: el disco no puede trabar a `Place`).
  - **3a:** `app` es lib + dos ejecutables: `wm.exe` (consola) y **`wmw.exe`** (subsistema GUI, D5); la CI revisa los dos. `wm` sin argumentos = guardián + core. **Instancia única** con mutex por sesión tomado antes de restaurar ventanas (un segundo `wm` le pide al primero que abra los ajustes y sale; un segundo core sale con 2). Log con fecha y hora local, una escritura por línea; el guardián y el hook de pánico escriben en `wm.log`; el guardián no relanza al apagar, pasa `WM_RELAUNCH` (gancho de D4) y sin consola avisa con un cuadro si se rinde. **Iniciar con Windows** (`HKCU\…\Run` → `wmw.exe`, respeta la marca del Administrador de tareas): opcional, desde el menú o `wm autostart on|off`, nunca solo. Arreglo de un despertar perdido del brain (Fable): `WAKE_PENDING` se rearma al empezar a leer los carriles.
  - **3b:** ícono dibujado en memoria (claro u oscuro según la barra; contorno en pausa), `Shell_NotifyIconW` por `dynlib`. Clic: ajustes; clic derecho: Ajustes, Pausar/Reanudar atajos, Deshacer el último movimiento, Iniciar con Windows ✓, Salir. `TaskbarCreated` lo vuelve a agregar; `WM_ENDSESSION` sale prolijo. Órdenes `wm quit|pause|resume|undo|ajustes` por el mensaje registrado `wm.command`. **Pausa:** el hook deja pasar todo y `Caps` vuelve a ser Bloq Mayús. **`Caps` + Esc ya no sale** (solo en modo prueba o si no hay ícono). **Aviso de administrador:** con una app elevada al frente, un globo de la bandeja (una vez por app, nunca dos en 60 s) [S: sin probar con una app elevada real].
  - **3c:** `apps.txt` se recarga solo ~100 ms después de guardarlo (espera de kernel sobre la carpeta, antirrebote para el temporal + renombrar).
  - **`ci-probe`:** su core usa su propia instancia (`WM_INSTANCE`) y su propia carpeta de datos (no toca la del usuario); sale con la orden `quit`; chequeos nuevos "bandeja" (pausa y reanudar con teclas reales) y "recarga" (~104 ms y la letra nueva funciona). 21 chequeos.
  - **Falta:** abrir los ajustes con el entorno limpio del usuario (`CreateEnvironmentBlock`; hoy sin las variables `WM_*`); probar a mano: reiniciar Explorer, iniciar sesión con el inicio automático, una app de administrador al frente.
- **Hito 1c (15/11/2026): hecho.** F = maximizar de Windows (`Effect::Maximize`, `ShowWindowAsync(SW_MAXIMIZE)`); repetir restaura (Windows recuerda el tamaño). C = centrar sin cambiar el tamaño; repetir alterna 60 % → 80 % → original (si ya estaba centrada, empieza en 60 %); el ciclo vale mientras la ventana siga donde la dejó C. Z = historial global de 32 entradas (D3): una entrada por gesto (un acorde se deshace con una sola Z), F y C también; las teclas opuestas que se anulan no dejan entrada; si la ventana ya no existe, salta a la anterior en la misma tecla; restaura también "maximizada". Un rectángulo restaurado o centrado que coincide con una mitad o esquina cuenta como esa ubicación para la tecla siguiente (error que encontró la sonda). 9 pruebas nuevas (102 en total). Con el core real (`ci-probe`): F, F, C, Z, Z, Z en 1,3-1,9 ms cada una. **Probado por el usuario** (F, C, Z y acordes, ~120 acciones, todas a 0 px). Encontró que desde maximizada una de cada tres C no hacía nada (el "original" era el 60 %): ahora cada C saltea los pasos que darían el mismo rectángulo. Notó que "la mitad de arriba" desde un costado ya no sale con W (costo aceptado de la v2): se llega con F y después W.
- **Hito 1b (15/11/2026): hecho.** `neighbor` (el monitor más cercano de ese lado, primero los que se solapan en el otro eje) y `mirror` (entra por el lado opuesto; una esquina conserva la otra componente). Empuja solo si la ventana ya está pegada al borde de ese lado (mitad o esquina de ese lado; "A siempre lleva a la izquierda", como `Win + ←`): desde la mitad derecha, `A` primero va a la mitad izquierda del mismo monitor. Una esquina de ese lado cruza conservando la otra componente. Sin vecino, no hace nada y lo dice. *Corregido tras la prueba del usuario:* la primera versión empujaba al repetir la misma tecla aunque la ventana estuviera del otro lado (empujar, volver y `A` otra vez decía "no hay otro monitor"). Los bordes se escalan al DPI de destino y la verificación espera a que la ventana informe el DPI nuevo. 9 pruebas nuevas (ida y vuelta, esquina, mitad puesta a mano, cruce a 150 % en una sola pasada). Con dos monitores reales (`ci-probe`): empujada al vecino en 2,0 ms y de vuelta en 2,6 ms. **Probado por el usuario con Brave**, también con el segundo monitor al 150 %: 0 px; al cruzar de escala Brave necesitó 2 pasadas (82-93 ms): los bordes escalados del primer cálculo no son exactos para Chromium.

Pruebas: **P** = puras (`cargo test`, en la CI), **C** = `wm ci-probe` ampliado (job `ventanas`, x64 obligatorio), **M** = manual del autor, **U** = con la usuaria.

Para C: mover el bucle a `app/src/brain.rs`, reutilizable por `core` y `ci_probe`, y manejarlo con `SendInput` real ampliando `session::send_hyper_chord` a secuencias (Caps + Alt + tecla). En ARM64 los escenarios con foco quedan informativos (§13.11).

| Hito | Qué entra | Hecho cuando | Pruebas |
|---|---|---|---|
| **0. Cimientos** | `brain.rs` + `World::step` con la capa de apps adentro; `Inspect`, `Scan` extendido, `WinKey`; señales del hook y máscara de direcciones; `catch_unwind`; log a archivo; plataforma falsa | `wm core` se comporta igual; CI verde | P: 10 000 secuencias de teclado con las señales. C: chequeo "inspect" |
| **1a. Mitades y esquinas** (instantáneo) | `place.rs`, `Combo`, colocación por marco con chequeos por plazo, "el último gana" | ≤ 1 px en títere, Brave y Terminal; tecla → `SetWindowPos` p99 < 10 ms; brain nunca > 5 ms | P: slots, combinación, tolerancia. C: A, D, W+A juntas y en secuencia. M: 10 min de uso |
| **1b. Empujar** | `neighbor`, cruce de DPI en dos pasadas | Ida y vuelta entre 2 monitores a 100 y 150 % con 0 px | P: nunca a sí mismo; ida y vuelta. M: dos monitores |
| **1c. F, C, Z** | Maximizar y volver, ciclo de centrar, diario | Tabla de §3.1 completa | P: deshacer(hacer) = identidad; C vuelve al original. C: `IsZoomed` ida y vuelta; Z × 3 vuelve al origen |
| **1d. Casos difíciles** | Títeres nuevos: tamaño mínimo, no redimensionable, lento, que se reubica, sin borde, sin DPI awareness | Tabla de §3.3 completa | P: simulación. C: cada títere + el colgable sin frenar al resto |
| **1e. Animación** (con tope) | Carril `anim`; vuelos del mismo tamaño; mini-spike 6b para redimensionar | `wm-bench ghost`: 0 cuadros defectuosos, ≥ 95 %. Si 6b no cierra en el tope, queda instantáneo | M + `wm-bench` |
| *1f. G/V (si D8)* | Agrandar o achicar 10 % anclado al borde exterior del slot | — | P + C |
| | **Usable: capa de ventana completa** | | |
| **2a. Mapa mínimo** | Plazo en el brain, hilo ui, mapa estático en el monitor del foco, todas las rutas de ocultado, `request_reset` | Aparece a 400 ms (+ ≤ 25); nunca en acordes rápidos; nunca queda pegado; Bloq Mayús intacto | P: `MapState`. C: Caps 500 ms → overlay visible + captura; toque → sin overlay. M: bloquear sesión; `Caps + F12` |
| **2b. Contenido real** | `map_view`, capa de ventana con el pulgar, tecla en `accent`, tokens, pie | La usuaria encuentra "centrar" en < 10 s (`keyboard-design.md:539`) | P: instantáneas de `MapView`. M: revisión visual |
| **2c. Pulido y medición** | Fundido, reducir animaciones, escala por DPI, recrear al cambiar monitores | ≤ 30 MB con el mapa; 0 % de CPU tras ocultarlo | `wm-bench` |
| | **Usable: se aprende sin documentación** | | |
| **3a. Arranque** | `wm` = guardián + core; sin consola (D5); instancia única; aviso al rendirse | Doble clic no abre terminal; la segunda instancia no duplica | C: lanzar `wm` → dos procesos y ventana `wm-ui`. M |
| **3b. Bandeja** | Ícono, menú (Ajustes, **Pausar**, **Deshacer**, Salir), `TaskbarCreated`, lanzar ajustes con entorno del usuario, **aviso de la bandeja** cuando los atajos no llegan (ventana de administrador al frente) | "Salir" → código 0 sin relanzar; matar el core → ícono de vuelta en ≤ 1 s; "Pausar" deja pasar todas las teclas | C: comando "salir" a `wm-ui`. M: reiniciar Explorer |
| **3c. Recarga** | Handle de cambios, antirrebote, `LoadBindings` | Guardar en ajustes rige en < 300 ms sin reiniciar | C: `APPDATA` temporal; escribir `q = wm` → Caps + Q enfoca un títere. M: 10 min en reposo con WPR, 0 cambios de contexto |
| | **Usable: se instala y se usa sin terminal** | | |
| **4.0 Verificaciones** (medio día) | (i) diálogo con dueño al esconder al dueño; (ii) Alt+Tab y clic en la barra sobre una ventana con cloak; (iii) `SetProp` y cloak sobre una elevada; (iv) foco al shell; (v) minimizada + cloak; (vi) una UWP; (vii) video a pantalla completa | Cada [S] de §4 pasa a [V] o cambia el diseño | M, anotado en `architecture.md` |
| **4a. Ir y llevar** | `desks.rs`, `wm.desk`, adopción, autotest de cloak, respaldo `minimize` | 6 títeres: `DWMWA_CLOAKED` y props coinciden con el modelo tras 50 cambios; cambio p95 < 30 ms | P: invariantes 1-3. C: escenario de escritorios |
| **4b. Integración** | `Caps + letra` entre escritorios; `FOREGROUND` de una escondida; `Reassert`; fila de escritorios en el mapa; Z de "llevar" | Sesión 1 completa en la PC del autor | P. C. M: barra, Alt+Tab |
| **4c. Recuperación** | Pánico → restaurar; relanzamiento (D4); `TaskbarCreated` → reconciliar; "Mostrar todas las ventanas"; salida limpia | `crash-test` contra el **core real** (hoy usa `hide-hold`): 50 muertes, 0 perdidas, < 500 ms | P: caída en cualquier punto. C: `crash-test core 50`. M: matar Explorer |
| **4d. Monitores** | Avisos por `wm-ui`, `StormTracker` con plazos, rescate (incluye escondidas), recrear overlays y fantasma | Guion de cables y bloqueo de §13.9 con el core corriendo: 0 ventanas fuera de pantalla | P: las 33 pruebas de `screens`. M |
| | **Usable: sesión 1 con la usuaria (U)** | | |
| **5. Mouse** | Equivalencias de `keyboard-design.md` §7 que no da Windows: **`Caps` + arrastrar** mueve desde cualquier parte de la ventana y **`Caps` + clic derecho + arrastrar** la redimensiona (hook de mouse **solo** con Hyper apretado, architecture.md §4.2); indicador de escritorios en la bandeja para ir y llevar (con el hito 4); arrastrar una ventana al indicador del escritorio | Mover y redimensionar sin apuntar al título; 0 % de CPU sin Hyper (sin hook de mouse permanente) | P: la máquina del arrastre. C: arrastre inyectado con el core real. M |

Lo que ya da Windows y no hace falta construir: arrastrar al borde o a la esquina (Snap), doble clic en el título para maximizar (coherente con D1), ir a una app desde la barra de tareas, pasar una ventana al otro monitor arrastrándola. Quedan para después de v1: arrastrar íconos sobre el mapa para asignar letras (pide íconos, D7) y el gesto del mouse para centrar ("a definir" en `keyboard-design.md` §7).

**Antes de la sesión U:**
- sembrar `apps.txt` con sus apps reales (la lista de los ajustes es fija: 40 apps más las del archivo);
- ocultar en los ajustes lo que todavía no hace nada: las tres opciones que no se guardan (§13.10) **y el selector "Apps primero / Ventanas primero"** (`settings/ui/settings.slint:138`; el perfil va después de v1);
- confirmar qué PC tiene ella: Windows 10 u 11, x64 o ARM64 (architecture.md §14.2, §13.11);
- probar el `wm.exe` sin firmar con Defender en su máquina (el hook de teclado se parece a un keylogger) y decidir cómo se le actualiza (§10);
- ~~actualizar `README.md`~~ hecho el 16/11/2026.

---

## 8. Riesgos y decisiones

### 8.1 Riesgos

| Riesgo | Mitigación |
|---|---|
| La animación con cambio de tamaño no está validada (hallazgo 5): la mayoría de los movimientos saldrían instantáneos | D6. Instantáneo es el valor por defecto de §1.8 |
| Diálogos con dueño, Alt+Tab y elevadas frente al cloak son [S] | Hito 4.0 antes de escribir escritorios. El diseño degrada a `sticky` |
| Una app en otro escritorio que roba el foco provoca un salto de escritorio | Aceptado en v1 (§6.3). El bloqueo de foco de Windows lo hace raro [S] |
| Con una ventana de administrador al frente no hay atajos ni mapa | Limitación conocida; UIAccess fuera de alcance |
| Las esquinas muestran un instante la mitad | Aceptado a cambio de latencia cero en las mitades; revisar en la sesión |
| Mapa pegado si se pierde `HyperUp` | Seis rutas de ocultado + tope de 30 s + invariante 7 |
| Explorer colgado al tocar la bandeja | `Shell_NotifyIconW` por el carril `shell`, con plazo |
| El foco en la CI ARM64 es inestable (§13.11) | Los escenarios con foco bloquean solo en x64 |
| Ruido de WinEvents si más adelante se registra `SHOW`/`DESTROY` | No en v1; medir con WPR cuando llegue el tiling |

### 8.2 Decisiones del usuario

**Decidido (usuario, 15/11/2026): todas según la recomendación.** D1 maximizar nativo; D2 escritorios globales; D3 diario global; D4 volver a esconder tras una caída; D5 segundo target sin consola; D6 instantáneo + mini-spike 6b con tope de 1-2 días; D7 el mapa sigue visible, solo texto; D8 se agregan G/V (hito 1f) y la sesión empieza por el mapa.

| # | Decisión | Opciones | Recomendación |
|---|---|---|---|
| D1 | Qué es F | (a) Maximizar nativo, con la animación propia de Windows. (b) Rect = área de trabajo | **(a).** El botón de maximizar, el doble clic en el título y arrastrar para restaurar siguen coherentes; "volver" es exacto y gratis |
| D2 | Escritorios | (a) Globales: todos los monitores cambian juntos. (b) Por monitor | **(a) en v1.** Es el modelo mental de Windows; los datos permiten pasar a (b) después |
| D3 | Alcance de Z | (a) Diario global. (b) Por ventana | **(a).** Es lo único que deshace un "llevar al escritorio 2" hecho por error |
| D4 | Tras una caída del core | (a) El core relanzado vuelve a esconder según `wm.desk`. (b) Todo visible en un solo escritorio | **(a).** Si el guardián se rinde, todo queda visible |
| D5 | Arranque sin consola | (a) Segundo target GUI con el mismo código. (b) Pasar `wm.exe` a GUI + `AttachConsole`. (c) Aceptar el destello de consola | **(a).** Unas 20 líneas, sin tocar CLI ni CI; +~1 MB en disco |
| D6 | Animación en esta versión | (a) Instantáneo, animar solo el mismo tamaño y dar a 6b un tope de 1-2 días. (b) Bloquear el hito 1 hasta animar todo | **(a)** |
| D7 | Mapa | Tras una acción con el mapa abierto: ¿sigue visible? ¿Íconos o texto? | **Sigue visible, con la tecla en `accent`; solo texto en v1** |
| D8 | Sesión 1 | La tarea "más ancho" no tiene atajo y la bienvenida no existe | **Agregar G/V como hito 1f** (~1 día [S]). Empezar la sesión por el mapa en lugar de la bienvenida |

**Menor:** agregar `proptest` como dependencia de desarrollo solo en `wm-model`, para las máquinas de estado de `World`.

---

## 10. Lo que prometen los documentos y no entra en v1

Auditoría del 16/11/2026 (`keyboard-design.md`, `design.md`, `architecture.md`, `findings.md`, `README.md` contra este plan), después de notar que las equivalencias con el mouse no estaban ni dentro ni fuera. Todo lo documentado tiene ahora un lugar: un hito (arriba), "fuera de v1" (línea de alcance) o esta lista.

**Ubicado en un hito:** arranque automático (3a), Pausar y Deshacer en la bandeja y aviso de "atajos no disponibles" con una ventana de administrador (3b), mouse (5), ocultar el selector de perfil y actualizar el README (antes de la sesión U).

**A decidir antes de las 1–2 semanas de uso real** (no bloquean la sesión 1):
- **Avisos en pantalla** cuando una tecla no hace nada (no hay otro monitor, la app no responde, pantalla completa): hoy quedan solo en el log. Para una persona no técnica, que no pase nada parece una falla (`architecture.md` §5.5, `design.md` §5). Recomendación: un aviso discreto en el mapa o la bandeja; decidir con la sesión 1.
- **Registro local de uso** con su permiso (`keyboard-design.md` §12.3): `wm.log` cubre una parte, pero rota por tamaño.
- **Instalación:** cómo se le actualiza y se desinstala; la firma del ejecutable va después de v1 (`architecture.md` §12.3–12.4, §9.7).

**Después de v1:**
- Enseñar en el momento ("esto también es `Caps` + …", `keyboard-design.md` §10.2).
- Borde de foco prendido por defecto (`keyboard-design.md` §14.7, `design.md` §5; con `DWMWA_BORDER_COLOR` cuesta poco).
- Atajos del nivel 2: foco con Hyper + flechas, X para cerrar (apagado por defecto), "llevar y seguir" con Shift (`keyboard-design.md` §6.3, §14.7).
- Teclado y configuración: elegir qué hace un toque de `Caps`, otra tecla Hyper desde la interfaz, detectar choques con PowerToys o AutoHotkey, perfiles de flechas y Vim, "letra = categoría", exportar e importar, el puntero que sigue al foco, WASD por posición física con las etiquetas reales de la distribución en el mapa (hoy QWERTY fijo, `mapview.rs`) (`keyboard-design.md` §4.3, §5, §6.4, §8.2, §10.1, §11.3, §14.7; `architecture.md` §5.7).
- Pausarse sola con apps a pantalla completa (`architecture.md` §5.7): hoy el mapa y C/F/Z/WASD/G/V la respetan, pero el hook sigue activo.
- Volcado de fallos local (`architecture.md` §9.10).
- Arrastrar íconos sobre el mapa y el gesto de centrar con el mouse (hito 5, nota).

**Descartado:** colores por grupo en el mapa (`keyboard-design.md` §10.1): choca con la regla de un solo acento de `design.md` §3.2.

**Ya hecho aunque el plan no lo nombraba:** la tecla máscara para "Alt antes de `Caps`" (`keyboard.rs`), Raw Input como vigía del hook y el hook que deja pasar todo si el brain no responde (`input.rs`).

---

## 11. La investigación (`findings.md` §5–§7, §14) contra lo que hay

Revisión del 16/11/2026, a pedido del usuario: qué de lo que la gente **ama**, **odia** y **pide** cubre v1, y qué no.

**Lo que odia (§6), cubierto en v1:** latencia (hook de ~10 µs, ir a una app en ~2 ms); configurar con archivos (ajustes con interfaz; `apps.txt` solo como respaldo); comportamiento errático (verificación por pasadas, no se pelea con la app, nunca pierde ventanas: guardián y marcas); apps que se resisten (tamaño mínimo y fijo, colgadas, lentas: 1d); varios monitores (empujar, cruce de DPI; tormentas en 4d); que todo cambie de tamaño al abrir (no hay tiling automático); perder libertad (las ventanas siguen libres; `wm` solo actúa con una tecla); unir varias apps (Hyper incluido, una sola app); requisitos que dan miedo (sin admin, sin APIs privadas en el núcleo); funciones que nadie pidió.

**Lo que ama (§5), cubierto en v1:** velocidad y cambio instantáneo de app por inicial (`Caps` + letra); ciclos al repetir (C: 60 % → 80 %); animaciones que ayudan a seguir la ventana (1e, opcionales); estética cuidada (Geist, tokens); instalar y que funcione sin admin (hito 3). Con el hito 5: mover y redimensionar desde cualquier parte (Swift Shift).

**Lo que pide y nadie hace (§7, ⭐ = mayor oportunidad), y dónde está:**

| Pedido | Estado |
|---|---|
| ⭐ 1–2. Zonas fijas con pilas de ventanas, híbrido con tiling (la propuesta de valor de §14.1) | **Fuera de v1** (línea de alcance) |
| ⭐ 21. Configurar el layout visualmente, con mouse | **Fuera de v1** (va con las zonas) |
| ⭐ 4. Reglas por app (lado preferido, tamaño, flotar, posición al abrir) | **Sin lugar hasta ahora** → propuesta abajo |
| ⭐ 10. Reaplicar el layout solo (al abrir una app, al cambiar monitores, al volver del reposo) | Parcial: 4d rescata ventanas fuera de pantalla; la persistencia por topología (spike 7) está fuera de v1 |
| ⭐ 14. Selector por ventana y por monitor con posiciones estables | **Sin lugar hasta ahora** → propuesta abajo |
| 13. Ventanas nuevas en el monitor del puntero y sin tapar a otras | **Sin lugar hasta ahora** → propuesta abajo |
| 11. Grupos de apps que se abren juntos | **Sin lugar hasta ahora** → propuesta abajo |
| 16–17. Escritorios con nombre; llevar una ventana con el teclado | Llevar: hito 4. Nombres: después de v1 |
| 25. Tecla Hyper configurable | Después de v1 (§10) |
| 26. Espacios entre ventanas (accesibilidad) | Sin lugar → propuesta abajo |
| 28. Cuartos arrastrando a las esquinas | Ya lo da Windows (Snap) |
| 30. Animaciones en cero | Hecho (`WM_ANIM=0`, "reducir animaciones"); falta el interruptor en los ajustes |

**Conclusión:** v1 es la **base de teclado** para la sesión con la usuaria (lo que pidió `keyboard-design.md` §12): resuelve casi todo lo que la gente odia, pero **todavía no tiene el diferencial** que la investigación encontró (zonas con pilas, reglas por app, layouts que se reaplican solos).

**Propuesta de hoja de ruta después de v1 (a confirmar con el usuario):**
1. **v1.1, reglas simples:** posición al abrir por app (lado o zona preferida), ventanas nuevas en el monitor del puntero y sin tapar a otras, espacios entre ventanas opcionales. Barato sobre lo que ya existe (`place`, `anchor`).
2. **v2, zonas:** áreas de trabajo por monitor con pila, definidas visualmente con el overlay (el editor de zonas de architecture.md §3), arrastrar a una zona con `Caps` o clic derecho, atajos por zona, redimensionado vinculado. Es la ⭐ principal.
3. **v2.x, recordar todo:** persistencia por topología (spike 7, ya diseñado y probado), reaplicar al conectar o desconectar monitores y al volver del reposo; grupos de apps.
4. **Después:** selector por ventana y por monitor; tiling automático por área; los diferenciales de §14.2.
