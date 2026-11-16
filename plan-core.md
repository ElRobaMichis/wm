# Plan de integración del core: primera versión para la usuaria

Plan armado con Fable 5.2 el 15/11/2026, después de los spikes 1–9. Objetivo: la versión con la que la usuaria de prueba hace la sesión 1 de `keyboard-design.md` §12. **[V]** = verificado en el código o en las mediciones de `architecture.md`; **[S]** = supuesto, a comprobar. Las decisiones del usuario están en §8.2.

**Alcance, en orden:** capa de ventana → mapa al mantener `Caps` → arranque normal (guardián, bandeja, recarga de `apps.txt`) → escritorios 1–4. **Fuera:** tiling y zonas, buscador de comandos, bienvenida, "Hyper + letra libre pregunta", UIAccess, macOS.

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

> **Versión 2 (15/11/2026), la vigente.** Tras la prueba del usuario con dos monitores, la semántica de W/A/S/D se rehízo con Fable 5.2 para que dependa de **dónde está la ventana** y no de la tecla anterior ni del tiempo. La tabla de abajo, "Primera vez / Repetir", queda como historia: la reemplaza esta.
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
| **3b. Bandeja** | Ícono, menú, `TaskbarCreated`, lanzar ajustes con entorno del usuario | "Salir" → código 0 sin relanzar; matar el core → ícono de vuelta en ≤ 1 s | C: comando "salir" a `wm-ui`. M: reiniciar Explorer |
| **3c. Recarga** | Handle de cambios, antirrebote, `LoadBindings` | Guardar en ajustes rige en < 300 ms sin reiniciar | C: `APPDATA` temporal; escribir `q = wm` → Caps + Q enfoca un títere. M: 10 min en reposo con WPR, 0 cambios de contexto |
| | **Usable: se instala y se usa sin terminal** | | |
| **4.0 Verificaciones** (medio día) | (i) diálogo con dueño al esconder al dueño; (ii) Alt+Tab y clic en la barra sobre una ventana con cloak; (iii) `SetProp` y cloak sobre una elevada; (iv) foco al shell; (v) minimizada + cloak; (vi) una UWP; (vii) video a pantalla completa | Cada [S] de §4 pasa a [V] o cambia el diseño | M, anotado en `architecture.md` |
| **4a. Ir y llevar** | `desks.rs`, `wm.desk`, adopción, autotest de cloak, respaldo `minimize` | 6 títeres: `DWMWA_CLOAKED` y props coinciden con el modelo tras 50 cambios; cambio p95 < 30 ms | P: invariantes 1-3. C: escenario de escritorios |
| **4b. Integración** | `Caps + letra` entre escritorios; `FOREGROUND` de una escondida; `Reassert`; fila de escritorios en el mapa; Z de "llevar" | Sesión 1 completa en la PC del autor | P. C. M: barra, Alt+Tab |
| **4c. Recuperación** | Pánico → restaurar; relanzamiento (D4); `TaskbarCreated` → reconciliar; "Mostrar todas las ventanas"; salida limpia | `crash-test` contra el **core real** (hoy usa `hide-hold`): 50 muertes, 0 perdidas, < 500 ms | P: caída en cualquier punto. C: `crash-test core 50`. M: matar Explorer |
| **4d. Monitores** | Avisos por `wm-ui`, `StormTracker` con plazos, rescate (incluye escondidas), recrear overlays y fantasma | Guion de cables y bloqueo de §13.9 con el core corriendo: 0 ventanas fuera de pantalla | P: las 33 pruebas de `screens`. M |
| | **Usable: sesión 1 con la usuaria (U)** | | |

**Antes de la sesión U:** sembrar `apps.txt` con sus apps reales (la lista de los ajustes es fija: 40 apps más las del archivo) y ocultar las tres opciones de los ajustes que todavía no se guardan (§13.10).

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
