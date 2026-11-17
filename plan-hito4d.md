# Hito 4d: monitores, diseño sobre el código

Diseño de Fable 5.2 (16/11/2026) sobre el código real después de los hitos 0–5 y v0.1.2. Completa la fila 4d de `plan-core.md` §7 (línea 454) y la fila "Cambio de monitores" de §4 (línea 280); retoma lo que `plan-hito4.md` §7 dejó para después. **[V]** = verificado leyendo el código o en las mediciones de `architecture.md` §13.9; **[S]** = supuesto. Nada de este documento se ejecutó. Fuera de alcance: persistencia por topología (`state.json`, `RelRect`, `match_monitors`), que es v2.x.

**Hecho cuando:** con el core corriendo, el guion de cables y bloqueo de §7.4 termina con 0 ventanas fuera de pantalla (también las de otros escritorios), el mapa y los avisos se ven bien después de cada cambio, y el brain nunca espera más de 5 ms.

## 0. Qué hay hoy y desvíos de `plan-core.md`

| # | Hallazgo | Evidencia |
|---|---|---|
| 1 | El brain no escucha ningún aviso de pantallas. `display_watch` no lo despierta, su `start()` espera a que el hilo arranque, y su único consumidor sondea cada 200 ms | [V] `display_watch.rs:78-86`, `:117-179`; `display_test.rs:108` |
| 2 | El único rescate es perezoso: al mostrar una escondida, con `GetWindowRect` y 96 DPI fijos. `restore_all` (guardián, `wm restore`) no rescata | [V] `hide.rs:443-456`, `:241-256`, `:533-543` |
| 3 | `Overlays::is_current` y `GhostHost::is_current` comparan los monitores de ahora contra **lo que guardamos al crear**, no contra dónde está la ventana de verdad. Si Windows movió nuestra ventana oculta al desconectar un monitor y después vuelve la misma geometría, dicen "vigente" con la ventana corrida [S: que Windows mueva ventanas ocultas] | [V] `overlay.rs:79-83`; `ghost.rs:159-162` |
| 4 | Con los monitores cambiados, **los avisos en pantalla se omiten** hasta que se muestre el mapa (que es quien rearma) | [V] `ui.rs:450-453`, `:362-368` |
| 5 | **Z puede perder una ventana:** `undo()` usa `entry.frame` absoluto como destino. Si ese monitor ya no está, `monitor_of` devuelve el más cercano pero el rectángulo sigue fuera de pantalla | [V] `world.rs:1855-1857`; `place.rs:339-352` |
| 6 | `placed` guarda un `Spot` con el monitor entero (límites, área, DPI), pero `locate()` solo compara los límites: si cambia el área de trabajo o la escala, la tecla siguiente calcula con el área vieja | [V] `world.rs:1350-1356`, `:2104-2106` |
| 7 | Una colocación en curso verifica contra un destino calculado antes del cambio; su segunda pasada volvería a pedirlo (pelea con Windows) | [V] `world.rs:1992-2003` |
| 8 | `PBT_POWERSETTINGCHANGE` está en el `match` pero nunca llega: nadie llama a `RegisterPowerSettingNotification` | [V] `display_watch.rs:27`, `:99`; búsqueda sin resultados |
| 9 | El hilo de entrada ya recibe bloqueo, desbloqueo y vuelta del reposo, y el brain ya aterriza vuelos y oculta el mapa con ellos | [V] `input.rs:929-939`; `brain.rs:705-721` |
| 10 | `StormTracker` pide 400 ms de calma; el arnés validado con cables usó **800 ms** | [V] `screens.rs:227`; `display_test.rs:29` |
| 11 | `screens.rs` tiene hoy 12 funciones de prueba; "las 33" de la fila 4d era el total del crate en el spike 7 | [V] conteo de `#[test]` |

**Desvíos:**
1. **Los avisos no van por la ventana `wm-ui`** (plan-core §2.1 y fila 4d) sino por el hilo propio de `display_watch`. Desde el hito 3, `wm-ui` vive en `wm-tray`: menú modal, `Shell_NotifyIconW` que tarda segundos con Explorer colgado, y un `proc` que aparta los mensajes anidados que no son suyos [V] `tray.rs:1-12`, `:363-397`. Los avisos de pantalla son mensajes *enviados*: en ese hilo llegarían tarde justo cuando Explorer está ocupado (reposo, desbloqueo).
2. **Tampoco por la ventana del hilo de entrada:** es `TIME_CRITICAL` y no asigna ni toma locks (architecture §4.2); `display_watch::send` hace las dos cosas.
3. **Sin `Op::Displays` + `Inspect` de cada una:** una sola orden `Survey` trae monitores y marcos de todas en la misma observación.
4. **La identidad de monitores (EDID, serie) no se usa en v1:** la clave de topología es la geometría (límites, área, DPI), que es lo que le importa al rescate. De `displays.rs` se reutiliza solo el control de coherencia.
5. **El fantasma no se recrea por aviso:** ya lo hace solo en el próximo vuelo [V] `anim.rs:603-611`; alcanza con que `is_current` diga la verdad.
6. **Las teclas no se bloquean durante la tormenta** (el `Freeze` del spike era "no mover ni guardar"): cada acción ya observa los monitores en el momento. Solo se apaga la animación.

## 1. Avisos: qué se escucha, dónde y cómo llegan

    wm-display-watch (hilo propio, ventana top-level oculta, GetMessage: 0 % en reposo)
        │  canal (qpc, DisplayEvent) + ops::wake(brain)      [mismo patrón que watch.rs:80-82]
        ▼
    brain: collect() → collect_displays() → feed(Event::Screen(e))        (µs, nunca bloquea)
        ▼
    World.screens (StormTracker) ── deadline() ──► input::wait(plazo)
        │ Effect::Survey{req}        → ops, carril win: Op::Survey   → Event::Surveyed
        │ Effect::Rescue{req, moves} → ops, carril win: Op::MoveMany → Event::Rescued
        │ Effect::ScreensChanged     → ui.screens_changed()
        └ Effect::Land / HideMap / Log / Notice / Scan (ya existen)

| Mensaje de Windows | `ScreenEvent` | Qué hace el modelo |
|---|---|---|
| `WM_DISPLAYCHANGE` (resolución, cantidad, escala) | `Changed` | **Congela** (§2) y avisa a la tormenta |
| `WM_SETTINGCHANGE` con `SPI_SETWORKAREA` | `WorkArea` | Avisa a la tormenta, sin congelar |
| `PBT_APMSUSPEND` | `Suspend` | Congela; `asleep = true` |
| `PBT_APMRESUMEAUTOMATIC` / `RESUMESUSPEND` | `Resume` | `asleep = false`; avisa |
| `WTS_SESSION_LOCK` / `UNLOCK` | `Lock` / `Unlock` | `locked`; al desbloquear, avisa |
| `WTS_CONSOLE_DISCONNECT`, `WTS_REMOTE_DISCONNECT` (0x4, nuevo) | `Disconnect` | `away = true` |
| `WTS_CONSOLE_CONNECT`, `WTS_REMOTE_CONNECT` (0x3, nuevo) | `Connect` | `away = false`; avisa |
| `WM_SETTINGCHANGE` genérico, `WM_DEVICECHANGE` | — | **No se reenvían en el core** |

- **Por qué no `dispositivos` ni `configuración`:** en las trazas reales siempre vinieron junto a `pantallas` y `área de trabajo` [V] §13.9. Solos son un pendrive, un auricular Bluetooth o un cambio de tema, y despertarían al brain por nada.
- **`display_watch.rs`:**
  - `Watch::start(brain: u32) -> Watch` no espera al hilo (como `Ui::start` y `Tray::start`; esperar el arranque de un hilo costó > 5 ms, `ops.rs:458-461`). El primer mensaje es `Ready(Result)`.
  - `Watch::drain()` junta los repetidos seguidos.
  - `send()` llama a `ops::wake` si hay brain.
  - El título de la ventana es `instance::mutex_name("display")`, para encontrarla desde afuera como a `wm-ui` (`tray.rs:173-175`).
  - `start()` queda igual para `wm display-watch`.
- **Brain:** campo `displays: Option<Watch>` creado en `Brain::new` (`brain.rs:169-201`) y `collect_displays()` dentro de `collect()`, después de `rearm_wake()` (`brain.rs:789-796`; el orden evita el despertar perdido de `ops.rs:440-447`).
- **Garantías:**
  - el hook no se toca;
  - el brain solo drena un canal y corre `World::step`;
  - todo lo que lee o mueve ventanas va por el carril `win` con plazo y abandono [V] `ops.rs:352-383`;
  - `Survey` solo usa lecturas que no envían mensajes (`DwmGetWindowAttribute`, `GetWindowRect`, `IsIconic`, `IsZoomed`, `GetPropW`), como `inspect` [V] `windows_list.rs:365-368`;
  - `MoveMany` usa `SWP_ASYNCWINDOWPOS`.
- **Costo:** un hilo y una ventana oculta más. No hay despertares nuevos del proceso: `wm-ui` y `wm-input-sink` ya son top-level y ya reciben esas difusiones [V] `tray.rs:335-351`, `input.rs:1092-1117`.

## 2. Tormenta: cuándo se reconcilia y qué pasa en el medio

`World` gana `screens: Screens` en `world_screens.rs` (`impl World`, como `world_desks.rs`):

    struct Screens { storm: StormTracker, locked, asleep, away, dark: bool,
                     asked: Option<(Req, Ask)>,          // Ask = Snap | Recheck | Now
                     last_key: u64, monitors: Vec<MonitorObs>,
                     rescued: HashSet<WinKey>, stubborn: HashSet<WinKey>,
                     hints: Vec<&'static str>, since_us: u64, retries: u8 }

- **Reloj:** `StormTracker` trabaja en ms; se le pasa `now_us / 1000`.
- **Plazo:** `World::deadline()` (`world.rs:779-787`) suma `storm.deadline() * 1000` **solo si `!blocked()`** (`locked || asleep || away || dark`). Sin eso, `tick` con `locked` re-arma la espera cada 400 ms (`screens.rs:266-271`) y el brain despertaría 2,5 veces por segundo durante todo el bloqueo.
- **`tick`** (`world.rs:1868`): si `storm.tick()` pide `Snapshot` → `Effect::Survey`. Es `Ask::Snap` si `storm.active()`, y `Ask::Recheck` si venía del control tardío (`Settled → Calm`, `screens.rs:279-284`). `active()` es el único agregado a `StormTracker`: `!matches!(state, Calm | Settled)`.
- **`Event::Surveyed`:**
  - clave = `topology_key(monitores)`; es 0 si la orden falló o la configuración es incoherente;
  - `Snap` → `storm.snapshot(ms, clave)`; con `Reconcile(k)` se reconcilia (§3);
  - `Recheck` → si la clave difiere de `last_key`, cuenta como aviso nuevo (como mucho 3 seguidos con clave 0: no hay bucle si `QueryDisplayConfig` falla siempre); si es igual, una pasada tardía de control (§3, R4);
  - una respuesta con otro `req` se ignora.
- **Tiempos:** último aviso + 400 ms → lectura → + 250 ms → lectura igual → reconciliar (~650 ms) → control a + 3 s. Tope de 10 s. Con cables reales, 3–5 avisos en ~0,5 s cayeron siempre en un solo ciclo [V] §13.9.

**En el medio:**

| Cosa en curso | Con `Changed`, `Suspend` o `Resume` (`World::freeze`) | Por qué |
|---|---|---|
| Vuelos | `Land` a cada uno | El anfitrión cubre la pantalla virtual vieja; la respuesta `Flown` que llega después se ignora sola (`world.rs:1491-1493`) |
| Colocaciones (`jobs`), empuje demorado, gesto, `keys` | Se olvidan | Sus destinos son de la topología vieja; las respuestas pendientes se descartan solas (`world.rs:1028-1034`, `:1905-1907`) |
| Mapa visible | `map.reset()` → `HideMap`, como un clic (`world_drag.rs:94-100`) | Su ventana tiene el tamaño del monitor viejo; al soltar y volver a mantener `Caps` se rearma |
| Arrastre con `Caps` | **Sigue** | Lee el cursor en vivo; cortarlo con el botón apretado sorprende. El rescate no toca esa ventana |
| Aviso en pantalla | Sigue; termina solo a los 2,2 s o al rearmar | |
| Borde de foco | Sigue | Recalcula todo en cada `update()` y sigue a la ventana por `LOCATIONCHANGE` [V] `focus_border.rs:212-259` |
| Escritorios (`Sync`) | Sigue | Mostrar y esconder no dependen de la geometría |
| Teclas nuevas | Se ejecutan, **sin animación** (`may_fly` suma `!screens.storm.active()`, `world.rs:1451`) | Un aviso posterior vuelve a congelar; el control a + 3 s cubre lo que quede |

`WorkArea`, `Unlock`, `Connect` y `Boot` no congelan: no rompen vuelos ni overlays.

## 3. Reconciliar

**Regla de fondo:** Windows mueve; nosotros solo rescatamos lo que quedó inalcanzable. Después de la calma, una sola vez, y nunca guardamos un "lugar deseado" al que volver.

**Se le deja a Windows:**
- sacar las ventanas de un monitor que se fue y devolverlas cuando vuelve: 18 ciclos de cable sin que el motor tuviera que mover nada [V] §13.9;
- re-maximizar las maximizadas;
- los rectángulos de restauración de las minimizadas [S];
- reescalar por `WM_DPICHANGED`;
- no se reaplican mitades ni esquinas tras un cambio de resolución (no hay tiling en v1; la tecla siguiente lo arregla).

### 3.1 Observación

`Op::Survey` (carril `win`, plazo 100 ms) → `windows_list::survey()`:
- `displays::topology()`: `monitors()` si cada camino activo de `QueryDisplayConfig` tiene su monitor, y si no, error. Es el control de `displays.rs:127-132`, sin leer el EDID del registro.
- `user_windows()`, que ya incluye las escondidas por nosotros [V] `windows_list.rs:48-51`, más por cada una `ghost::frame_bounds`, `ghost::window_rect` y `hide::is_maximized`. El marco se lee bien con cloak [V] `anim.rs:677`.
- El brain lo traduce a `Survey { monitors: Vec<MonitorObs>, wins: Vec<SurveyWin { info: WinInfo, frame, outer, maximized }>, fg }`.

### 3.2 Plan puro

`screens::rescue_plan(monitores, vistas) -> Vec<(WinKey, Verdict)>` con `Verdict = Move { dx, dy } | Remaximize { en: Rect }`:

| Ventana | Qué se hace |
|---|---|
| Alcanzable (`reachable` sobre las áreas de trabajo, `MinVisible::at_dpi(máximo DPI)`) | Nada |
| Minimizada | Nada: "las minimizadas no cuentan" (§13.9). Red opcional en 4d.7 |
| Pantalla completa (marco == límites de un monitor) | Nada, aunque haya una barra arriba |
| Maximizada con el centro dentro de algún monitor | Nada. Así no depende de cómo informe DWM el marco de una maximizada [S] |
| Maximizada perdida | `Normalize` + `Place` centrada en el monitor más cercano + `Maximize`, la misma secuencia que ya usa `undo` (`world.rs:1821-1845`) |
| Normal, visible o **escondida con cloak en otro escritorio** | `Move` **entera** adentro del área del monitor más cercano (implementación, 16/11/2026: el desplazamiento mínimo de `screens::rescue` la dejaba apenas asomada, 120 × 48 px, y eso confunde a quien no sabe arrastrarla); el tamaño no cambia. `SetWindowPos` funciona con cloak [V] plan-core §4 |
| Con colocación en curso (incluye en vuelo), en arrastre, fuera de `WM_SCOPE`, o `stubborn` | Nada (`skip`). Para el `ci-probe`, el alcance garantiza que solo se muevan sus títeres |

**Límite:** los diálogos con dueño no se revisan (no están en `user_windows`); Windows los mueve con su dueño [S].

### 3.3 `World::reconcile(survey, clave)`

1. Bloqueada → no hace nada; el desbloqueo vuelve a avisar.
2. Sin datos (clave 0 por el tope de 10 s) → "no pude leer las pantallas; no muevo nada".
3. **Si la clave cambió:**
   - se vacían `placed`, `centers`, `sizing`, `limits`, `gesture` y `deferred` (son memorias de "repetir"; perderlas equivale a "primera vez", `plan-core.md` §3.3) y `stubborn`;
   - `Effect::ScreensChanged`, salvo en la primera lectura del arranque;
   - se guardan `last_key` y `monitors`.
4. **Rescate:** `Effect::Rescue { req, moves: Vec<(WinKey, x, y)> }` con la esquina nueva del rectángulo exterior (`outer + (dx, dy)`) → `Op::MoveMany`. Es un solo viaje de carril, con `SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_ASYNCWINDOWPOS`, y verifica el pid por ventana como `SetDesks` (`ops.rs:571-584`). Sin `jobs` ni pasadas de verificación.
5. **R4, una vez por ventana y por tormenta:**
   - `rescued` se vacía al empezar cada tormenta;
   - en el control a + 3 s, lo que esté fuera y no se rescató (la ola tardía de Windows) se rescata;
   - lo que ya se rescató y sigue fuera pasa a `stubborn` con una línea de log (app colgada, que aplicará el pedido al despertar, o app que se reubica sola). No se insiste.
6. **Escritorios:** `start_sync(Goal::Verify)` (R3 de `plan-core.md` §1.3). Si hay un `Sync` en curso queda en cola sin pisar un pedido del usuario [V] `world_desks.rs:131-142`. El rescate no espera al `Sync`: mover y mostrar o esconder conmutan. El `rescue_lost` de `show_many` queda como segunda red.
7. **Log** (una línea por tormenta) y aviso en pantalla si hubo rescates (`notices.rs`: "Traje N ventanas que habían quedado fuera de la pantalla"). `ScreensChanged` sale antes que `Notice`, para que el aviso encuentre las ventanas ya rearmadas.

### 3.4 Otros disparadores

- **Arranque:** `Event::Boot` cuenta como aviso (arranque = recuperación, architecture §9.5). El camino se ejercita siempre y el `ci-probe` puede probarlo sin ningún aviso de Windows.
- **Una acción del hook con `blocked()`:** limpia las banderas y avisa. Una tecla con `Caps` prueba que la sesión está desbloqueada; cubre un `Unlock` perdido.

### 3.5 Deshacer (Z) y monitores que ya no están

Las entradas del diario no se borran: el monitor puede volver, y entonces Z es exacta. Se corrige al usarlas.
- **`undo()`:** si `entry.frame` no es alcanzable con los monitores de esa observación, el destino pasa a `fit_inside(entry.frame, área del monitor más cercano)`, achicado si no entra (`centered`), y el log lo dice.
- **`go()`:** la misma red para todo destino (invariante 5 de `plan-core.md` §1.4): ninguna acción nuestra puede dejar una ventana fuera de pantalla.
- Una entrada "maximizada" ya usa el monitor más cercano (`world.rs:1824-1825`). Las de "llevar" (`Entry.desk`) no dependen de monitores.

## 4. Recursos por monitor

| Recurso | Qué guarda | Hoy | Con 4d |
|---|---|---|---|
| Overlays del mapa y del aviso (`Overlays.screens`) | Una ventana por monitor, del tamaño de sus límites | Se rearman solo al mostrar el mapa | `is_current` veraz (también `GetWindowRect` de cada ventana). `State::ensure_current()` (termina el aviso, oculta, `rebuild_screens`, actualiza `WINDOWS`, `built = None`, `shown = None`) lo usan `show`, `notice` (ya no se omite) y el nuevo `Cmd::Screens`. Responde `UiEvent::Screens { n, ms }` → "Mapa y avisos rearmados para N monitores" |
| Anfitrión del fantasma | Origen y tamaño de la pantalla virtual | Se recrea en el próximo vuelo | Solo `is_current` veraz. Durante la tormenta no hay vuelos |
| Borde de foco | Nada (lee marco, DPI y monitores en cada `update`) | Sigue por `LOCATIONCHANGE` | `Cmd::Screens` llama además a `border.update()`, por si una app sin DPI awareness no avisa [S] |
| `frame_period`, `pick_monitor`, `fullscreen_on` | Nada: leen en el momento | — | — |
| Modelo | `placed`, `gesture`, `deferred`, `jobs`, `centers`, `sizing`, `limits`, diario | — | §2 y §3 |
| Ícono de la bandeja | Tamaño `SM_CXSMICON` al dibujarlo | Explorer lo escala | Sin cambios (después de v1) |
| `drag::Session` | `Bounds::DEFAULT`, sin monitores | — | — |

## 5. Bloqueo, reposo y RDP

| Caso | Qué cambia | Qué no cambia |
|---|---|---|
| **Bloqueo** (`Win` + `L`) | `locked`: no se lee ni se mueve nada, sin timers. Los avisos que lleguen (un monitor DisplayPort que se apaga al minuto) quedan anotados. Al desbloquear: aviso → una reconciliación. El hilo de entrada ya reinicia el teclado y reinstala el hook | Escritorio activo, cloaks, marcas, diario. `Goal::Verify` al reconciliar re-esconde si Windows destapó algo |
| **Reposo** | `Suspend` congela (nada queda a medio hacer durante el sueño) y `asleep`. `Resume` avisa; lo normal es que la reconciliación espere al desbloqueo. Sin `state.json` no hay nada que guardar en los 2 s que da Windows | — |
| **Modern Standby** (portátiles nuevos) | No manda `Suspend` ni `Resume` [S]. Lo cubren `Unlock`, `Changed` y la regla de la tecla (§3.4) | — |
| **Pantalla apagada por inactividad** | Si el monitor se desconecta, es una tormenta común; al volver, otra. Opcional (4d.7): `RegisterPowerSettingNotification(GUID_CONSOLE_DISPLAY_STATE)` → `dark`, para no reconciliar contra el monitor de mentira que Windows inventa cuando no queda ninguno [S] | — |
| **RDP / cambio de usuario** | `Disconnect` → `away`. `Connect` → aviso. El `WM_DISPLAYCHANGE` de la sesión remota congela; el rescate trae las ventanas a la pantalla remota y, al volver a la consola, a la local | No probado: [S] |
| **Core relanzado con la sesión bloqueada** | Arranca con `locked = false` y puede reconciliar bloqueado. Es inofensivo (solo rescata lo inalcanzable) [S] | — |

El spike no pudo probar el reposo (la PC del autor no suspende): sigue en [S] y va en el guion manual.

## 6. Modelo y plataforma

**`wm-model` (puro):**
- **`screens.rs`:** `ScreenEvent`; `topology_key(&[MonitorObs]) -> u64` (hash de límites, área y DPI ordenados; 0 si vacío o con rectángulos vacíos; nunca 0 si es válido); `Seen { key, frame, minimized, maximized, skip }`; `Verdict`; `rescue_plan`; `StormTracker::active()`.
- **`world.rs`:**
  - `Event::{Screen(ScreenEvent), Surveyed { req, result: Result<Survey, String> }, Rescued { req, failed }}`;
  - `Effect::{Survey { req }, Rescue { req, moves }, ScreensChanged}`;
  - `Pending::{Survey(Ask), Rescue}`;
  - ajuste en `undo` y `go`; `deadline`, `tick` y `may_fly`;
  - `screen_stats()` para el resumen de salida;
  - el `match` exhaustivo de la `Sim` de sus pruebas (`world.rs:2213-2272`).
- **`world_screens.rs`** (nuevo): `Screens`, `on_screen`, `screens_tick`, `on_surveyed`, `freeze`, `reconcile`, `on_rescued`.
- **`notices.rs`:** el aviso de rescate.

**`wm-platform`:**
- `display_watch.rs` (§1, más `poke(nombre, mensaje)` para pruebas);
- `displays.rs` (`topology`);
- `windows_list.rs` (`survey`, y `move_async` compartido con `hide.rs:663` y `drag.rs:289`);
- `ops.rs` (`Op::Survey`, `Op::MoveMany`, `Output::Survey`, carril `win`, plazos de 100 ms);
- `overlay.rs` y `ghost.rs` (`is_current`);
- `ui.rs` (`ensure_current`, `Cmd::Screens`, `Ui::screens_changed`, `UiEvent::Screens`);
- `hide.rs` (4d.6: `rescue_lost` con las áreas calculadas una vez, también en `restore_all`).

**`app`:**
- `brain.rs` (`displays`, `collect_displays`, `Wait::{Survey, Rescue}`, tres efectos);
- `core.rs` (resumen);
- `ci_probe.rs` (chequeo "pantallas");
- `lib.rs` (`wm display-poke`, `wm fuera`).

### Pruebas puras

En `screens.rs`:
1. `topology_key` no depende del orden, cambia con cualquier campo, y da 0 solo sin monitores.
2. Si todo es alcanzable, el plan sale vacío: 5 000 escenas al azar con 1–3 monitores, coordenadas negativas y DPI mezclados.
3. Aplicar el plan deja todo alcanzable, es idempotente y no cambia ningún tamaño.
4. Minimizadas, pantalla completa con barra arriba, maximizadas con el centro adentro y `skip` no se tocan.
5. Una maximizada perdida da `Remaximize`.
6. `active()` vale solo entre el primer aviso y la reconciliación.
7. Las 12 pruebas actuales siguen sin tocarse.

En `world_screen_tests.rs`, con una plataforma simulada que tiene monitores, marcos, escritorios y un "Windows" que a veces mueve las ventanas solo:

8. N avisos en 300 ms dan un par de lecturas y una reconciliación; al final `deadline() == None`.
9. Bloqueada: ninguna lectura y ningún plazo de tormenta; al desbloquear, una reconciliación.
10. La clave cambia entre lecturas: sin rescate, espera otra calma. El tope de 10 s corta.
11. `Survey` con error: nunca se mueve nada; a los 3 reintentos tardíos se deja.
12. Una escondida del escritorio 2 fuera de pantalla se rescata sin `Show`; `Caps` + 2 la muestra adentro; visible ⇔ `desk == activo`.
13. Vuelo en curso + `Changed`: `Land`, sin `Place` al destino viejo, el `Flown` tardío se ignora.
14. Empuje demorado + `Changed`: nunca sale.
15. Mapa visible + `Changed`: `HideMap`. 10 000 secuencias con avisos intercalados conservan las invariantes del mapa.
16. Z con el monitor de la entrada desconectado: destino alcanzable. Propiedad: todo `Place` y `Fly` de `go` es alcanzable con los monitores de su observación.
17. `placed` tras cambiar solo el área de trabajo o la escala: la tecla siguiente usa lo nuevo.
18. `Sync` en curso al reconciliar: el rescate sale igual y `Verify` no pisa la tecla en cola.
19. R4: una app que se vuelve a ir no se rescata dos veces (una línea de log); la ola tardía sí se rescata en el control.
20. No pelear: lo que "Windows" movió antes de la lectura no recibe `Rescue`, y si lo mueve después no se lo devolvemos.
21. La ventana en arrastre no se rescata.
22. `Boot` da una pasada.
23. Una tecla con `locked` trabado destraba.
24. Propiedad general (10 000 secuencias de teclas, escritorios, avisos, bloqueo, reposo, monitores que van y vienen): en reposo y sin bloqueo, toda ventana no minimizada dentro del alcance es alcanzable, nada queda en vuelo, y nunca sale `Rescue` con `blocked()`.

## 7. Cómo se prueba sin tocar las pantallas del usuario

El agente que implementa **no** cambia la configuración de pantalla, ni desconecta, bloquea o suspende la PC del usuario.

### 7.1 `cargo test`

Todo §6. Es la mayor parte: la tormenta, el bloqueo, qué se rescata y qué no, Z y las invalidaciones son lógica pura.

### 7.2 `wm ci-probe`, chequeo "pantallas"

Seguro en la PC del usuario y en el runner. El core de la sonda corre con `WM_SCOPE=wm` (`ci_probe.rs:543-551`): solo puede mover sus títeres. Nada cambia en Windows: se postean mensajes a **nuestra** ventana `wm-display-watch`, como ya hizo el spike a mano [V] §13.9.

1. Títere P visible, títere Q llevado al escritorio 2 (con cloak), títere R en un lugar normal.
2. La sonda corre P y Q fuera de la pantalla virtual con `SetWindowPos` (son sus ventanas).
3. Cinco `WM_DISPLAYCHANGE` falsos a 100 ms. Se espera:
   - nada antes de ~600 ms;
   - a los ≤ 2 s, P y Q alcanzables, Q sigue con cloak, R no se movió;
   - una sola línea "Pantallas: en calma".
4. `Caps` + 2 → Q visible y adentro.
5. Bloqueo simulado: `WM_WTSSESSION_CHANGE` con `WTS_SESSION_LOCK` posteado a esa ventana [S: es posteable; no lleva punteros]; P se corre afuera y se manda otro `WM_DISPLAYCHANGE`; 1,5 s sin movimiento; `UNLOCK` → rescatada. Solo afecta a la tormenta: la ventana del hilo de entrada no se entera.
6. Arranque: P afuera antes de lanzar el core → rescatada sin ningún aviso (§3.4).

Suma ~6 s; revisar el tope de 90 s del vigía (`ci_probe.rs:60`).

**Lo que el runner puede y no puede:**
- **Cambiar la resolución, probablemente sí en `windows-latest`** (x64; es común hacerlo con `Set-DisplayResolution` o `ChangeDisplaySettingsExW`), **sin verificar** [S]. En `windows-11-arm`, desconocido.
  - Va en 4d.8, informativo, y solo con `GITHUB_ACTIONS=true` **y** `WM_PROBE_DISPLAY=1` puestos en `ci.yml`; nunca corre en una PC.
  - Con `CDS_FULLSCREEN` no se guarda en el registro, y la guarda `Cleanup` restaura.
  - Daría un `WM_DISPLAYCHANGE` y un área de trabajo reales en un monitor: `rebuild_screens`, el anfitrión del fantasma, y el mapa y un aviso después del cambio.
- **Agregar un monitor virtual: no.** Hace falta instalar un driver de pantalla indirecta (IddCx). No se planea.
- **Bloquear o suspender el runner: no.** Rompería el resto de la sonda. Solo simulado.
- **Queda sin cubrir por máquinas:** varios monitores, escalas mezcladas, cables, reposo (architecture §10 y §13.11). Para eso está el guion.

### 7.3 Herramientas

- **`wm fuera`:** solo lee (`survey` + `rescue_plan`) y dice "0 ventanas fuera de pantalla" o las lista por app, sin títulos. El agente la puede correr siempre.
- **`wm display-poke`:** postea un `WM_DISPLAYCHANGE` falso al core que esté corriendo. Para el autor.

### 7.4 Guion para el usuario

Unos 15 minutos, con `wm` corriendo normal.

**Antes:** abrí tres ventanas (por ejemplo el navegador, el Bloc de notas y el Explorador de archivos). Con dos monitores, dejá una en cada uno. Poné una en el segundo monitor y mandala al escritorio 2 (`Caps` + `Alt` + `2`).

1. **Cable.** Desenchufá el segundo monitor y contá hasta 5. ¿Están todas las ventanas en el monitor que queda? Apretá `Caps` + `2`: ¿aparece la que habías mandado ahí? Volvé con `Caps` + `1`. Enchufá de nuevo y contá hasta 5. Hacelo 5 veces, y 3 veces más apagando el monitor con su botón.
2. **Bloqueo.** `Win` + `L`, esperá 5 segundos y desbloqueá. 10 veces. Una de ellas, desenchufá el monitor mientras está bloqueada.
3. **Reposo** (si tu PC lo permite). Inicio → Suspender, y despertala. 3 veces; una, desenchufando el monitor mientras duerme.
4. **Escala.** Configuración → Pantalla → Escala: 125 % y de vuelta a 100 %. Mantené `Caps`: el mapa tiene que verse entero y centrado. Hacé algo que no pueda hacerse (con un solo monitor, `Caps` + `Alt` + `A` dos veces): tiene que aparecer el cartelito abajo.
5. **Al final.** Clic en el ícono de wm → "Mostrar todas las ventanas". ¿Falta alguna?

**Qué mirar en `%LOCALAPPDATA%\wm\wm.log`:**

| Línea | Qué significa |
|---|---|
| `Pantallas: cambio detectado (…); espero a que se calme.` | Una por tormenta |
| `Pantallas: en calma tras 0,9 s (pantallas, área de trabajo ×2): 2 monitores; 14 ventanas revisadas, 0 fuera de pantalla.` | **Lo esperado.** Una por cada cambio del guion |
| `… 1 rescatada: notepad (−1380, 0).` | Bien si era la del escritorio 2 o si Windows la dejó afuera. **Avisar** si es una ventana visible justo después de reconectar un monitor: sería pelear con Windows, y habría que subir `QUIET_MS` a 800 (hallazgo 10) |
| `Pantallas: sesión bloqueada; reviso al desbloquear.` | Bien. Nunca tiene que haber un "rescatada" entre esta y el desbloqueo |
| `Pantallas: la configuración siguió cambiando; espero otra calma.` | Bien si es ocasional |
| `Pantallas: brave sigue fuera de pantalla después del rescate; no insisto.` | **Avisar** |
| `Mapa y avisos rearmados para 2 monitores (4,2 ms).` | Una por cambio de monitores o de escala |
| Al salir: `Pantallas: 31 cambios, 2 ventanas rescatadas.` | El resumen |

Lo de siempre: el máximo del hook de teclado por debajo de 5 ms y ninguna línea de "hilo abandonado".

## 8. Pasos y recortes

Un commit cada uno; cada paso cerrado se anota en `plan-core.md` §7.

| Paso | Qué entra | Pruebas | Comportamiento |
|---|---|---|---|
| **4d.1** Modelo puro | `screens.rs` (`ScreenEvent`, `topology_key`, `Seen`, `Verdict`, `rescue_plan`, `active`); ajuste de destino en `undo` y `go` | P 1–7, 16 | Solo cambia Z hacia un monitor que no está |
| **4d.2** Plataforma | `displays::topology`; `windows_list::{survey, move_async}`; `Op::{Survey, MoveMany}`; `display_watch::Watch` (despierta, filtra, RDP, no espera); `is_current` veraz en overlays y fantasma; `ui::ensure_current` + `Cmd::Screens` | C: `survey` de los títeres igual a `frame_bounds`; `MoveMany` mueve un títere sin cambiarle el tamaño | Los avisos en pantalla dejan de omitirse tras un cambio |
| **4d.3** Tormenta, **solo observa** | `world_screens.rs`: eventos, `StormTracker` con plazos, banderas de sesión, `freeze`, invalidación de `placed` y demás, `ScreensChanged`. Brain: `collect_displays`, `Wait::Survey`. `reconcile` calcula el plan y lo **loguea** ("N fuera de pantalla"), sin mover | P 8–11, 13–15, 17, 23 | Se ve la tormenta en el log con riesgo cero de mover ventanas. Se puede dejar unos días |
| **4d.4** Rescate | `Effect::Rescue`, `Event::Rescued`, R4, `Goal::Verify`, pasada de arranque, aviso en pantalla, resumen de salida. `wm fuera`, `wm display-poke`. Chequeo "pantallas" | P 12, 18–22, 24. C: §7.2 | **4d funciona** |
| **4d.5** Documentación | `plan-core.md` §7 (avance y fila 4d), README, puntero desde architecture §13.9, guion de §7.4 | M: el guion | — |
| *4d.6* (opcional) | "Mostrar todas las ventanas" vacía `stubborn` y reconcilia ya (`Ask::Now`); `rescue_lost` en `restore_all` (guardián y `wm restore`); rescate al soltar un arrastre con `Caps` (el recorte de `plan-hito5.md`) | P + C | El botón de pánico también trae lo que quedó afuera |
| *4d.7* (opcional) | `GUID_CONSOLE_DISPLAY_STATE` → `dark`; minimizadas "sin revisar" (se miran al pasar al frente o al mostrarlas; cubre el respaldo `minimize`) | P | — |
| *4d.8* (opcional) | Cambio real de resolución en el runner x64, informativo | C | — |

- **Imprescindible para la sesión 1:** 4d.1–4d.4, más el guion al menos una vez en la PC del autor. Muy recomendable: el "Mostrar todas" de 4d.6, que es lo que ella va a tocar si pierde una ventana.
- **Recortable sin riesgo:** 4d.7 y 4d.8. Si el tiempo no da, 4d.3 puede fusionarse con 4d.4.

## 9. Riesgos y supuestos abiertos

| Riesgo | Mitigación |
|---|---|
| 650 ms puede ser poco: rescatar algo que Windows iba a acomodar (hallazgo 10) [S] | El rescate es mínimo, solo de lo inalcanzable, y no recordamos destinos. 4d.3 deja medirlo antes de mover nada. Salida prevista: `QUIET_MS = 800` y ajustar los tiempos de 3 pruebas |
| Una app que estaciona su ventana fuera de pantalla a propósito | Se la trae una vez por tormenta; si se vuelve a ir, queda en `stubborn` hasta el próximo cambio de topología o "Mostrar todas" |
| `QueryDisplayConfig` lento durante un cambio de modo: se abandona el hilo del carril `win` [S] | Se llama solo tras 400 ms de calma; plazo de 100 ms; el fallo vale como clave 0 (no se mueve nada) y se reintenta |
| Reposo y Modern Standby sin probar | Guion; la regla de la tecla destraba (§3.4) |
| Posteo de `WM_WTSSESSION_CHANGE` entre procesos [S] | Si no llega, el bloqueo queda cubierto solo por las pruebas puras 9 y 23 y por el guion |
| `rebuild_screens` nunca corrió con un cambio real | 4d.8, o el paso 4 del guion |
| Windows 10 | El "recordar ubicaciones por monitor" es de Windows 11: en 10 habrá más rescates reales, y es justo el caso para el que existe 4d [S] |

## 10. Ajustes tras la prueba del usuario (16/11/2026)

Prueba con cables y bloqueo: todo bien (Windows acomodó las visibles; se rescató solo la escondida del escritorio 2; nada se movió con la sesión bloqueada). Apagar el monitor con su botón no le avisa nada a Windows en esa PC (no aplica). Dos ajustes, con diseño de Fable 5.2:

1. **Cambio de escala o de área de trabajo:** las ventanas que pusimos en una mitad o esquina **y siguen ahí** (`screens::still_there`: a ≤ 16 px de donde quedaron, o con otra escala, centro en la ubicación nueva y lados ±10 % de lo escalado) se **reencajan exactas** en la misma ubicación del mismo monitor, una vez tras la calma, sin Z ni vuelo (`World::resnap`). Cambia la regla de §3 ("no se reaplican mitades"): se reaplican solo las que pusimos nosotros y nadie movió. Antes, Brave quedaba despegada y A decía "no hay otro monitor" (a 1 px contaba como la mitad).
2. **Una tecla a 1 px de su lugar, sin vecino:** la deja exacta (además del cartel), sin entrada en Z; con vecino empuja como siempre.
3. **Escondida rescatada que vuelve:** al rescatar se guarda de dónde salió (con la topología de antes); si esa topología vuelve y nadie la movió, vuelve a su lugar (`return_home`). Windows lo hace solo con las visibles.

**Segunda prueba del usuario (16/11/2026):**
- Rayo no se reencajó la primera vez: la había puesto en su esquina **otra sesión** de `wm` (la memoria `placed` no sobrevive). Ahora cada reconciliación adopta como "puestas" las ventanas que están exactas en una mitad o esquina (`adopt_slots`).
- Claude, pegada a la izquierda en el monitor 2, volvía despegada tras desconectar y reconectar: la memoria se perdía porque su monitor no estaba. Ahora lo que no se puede decidir (monitor ausente, minimizada, algo en curso) se conserva para el próximo cambio, y el control tardío también reencaja (Windows restaura algunas después de la calma).
- `ardia` volvió a su monitor aunque la habían pasado a la mitad derecha: la casa se comparaba solo por la esquina (era la misma). Ahora por el marco entero.
- Lo que hace Windows y no tocamos: al desconectar, Windows 11 **minimiza** las ventanas de ese monitor o las **lleva** al otro (opción "Minimizar ventanas cuando se desconecta un monitor"), y al reconectar devuelve las visibles a su lugar de antes ("Recordar ubicaciones de ventanas…"), aunque se hayan movido mientras tanto.

**Tercera prueba (16/11/2026):** Claude vuelve bien a su mitad al reconectar. Brave no se reencajó al volver de 125 % a 100 % (Chromium reescala a su manera): `still_there` acepta además la ventana que sigue pegada a la esquina de su ubicación con un tamaño razonable (±35 %), y el log dice qué vio cuando no reencaja una. **Decisión del usuario: que decida Windows.** Al reconectar, Windows devuelve cada ventana a su lugar en ese monitor, también las escondidas en otro escritorio y aunque se hayan movido mientras tanto ("Recordar ubicaciones de ventanas…"); se quitó "devolver a su monitor" (las casas del punto 3 de arriba): repetía lo que hace Windows y dos programas moviendo la misma ventana es un riesgo sin beneficio.
