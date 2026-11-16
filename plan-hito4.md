# Hito 4: escritorios 1–4, diseño sobre el código

Diseño de Fable 5.2 (16/11/2026), hecho sobre el código real después de los hitos 0–3 y con los resultados de `wm desk-check` (§6). Completa y en parte reemplaza `plan-core.md` §4; cada paso cerrado se anota en `plan-core.md` §7.

## 0. Desvíos de `plan-core.md` §4

1. **Una sola transacción, `Sync`:** Scan → plan puro → Mostrar → Foco → Esconder. Ir, llevar, `Caps + letra` a otro escritorio, clic en la barra, `UNCLOAKED`, `TaskbarCreated`, D4 y deshacer un "llevar" son el mismo camino con distinto `Goal`. El plan sale de lo **observado** en el Scan: cada cambio también reconcilia y no hacen falta reintentos.
2. **`want_hidden` no se guarda:** es `desk != activo && !sticky`.
3. **Sin `Op::Reassert`:** `Hider::hide` pasa a ser idempotente por observación (arregla el hallazgo 8).
4. **El modelo no conoce el método** (cloak o minimize): vive en la plataforma.
5. **Sin autotest sobre una ventana propia** (haría falta una ventana visible con botón en la barra): sonda de la interfaz al arrancar y confirmación en el primer uso.
6. **Regla de "rebote"** en el salto por `FOREGROUND` (§3).
7. **D4 solo con `WM_RELAUNCH ≥ 1`:** un arranque normal deja todo en el 1 y borra `wm.desk`.
8. **4d recortado** a un rescate perezoso al mostrar.
9. **`WM_SCOPE` para el `ci-probe`:** sin eso, su core escondería las ventanas reales del usuario al probar `Caps + 2`.

## 1. Teclas

No hay que tocar `keyboard.rs` ni `input.rs`: el hook ya emite `HyperAction::Digit { digit, thumb }` (fila superior; el teclado numérico cae en `Other`) y `world.rs` solo escribe "pendiente". Dígitos fuera de 1–4: log. Un dígito pone `self.keys = None` y, con el mapa visible, destella su tecla.

## 2. Modelo: `wm-model/src/desks.rs` (puro)

```rust
pub struct Desks { active: u8, wins: HashMap<WinKey, DeskWin>,
                   last_focus: [Option<WinKey>; 4], last_fg: Option<WinKey>, scope: Option<String> }
struct DeskWin { desk: u8, sticky: bool, strikes: u8 }
pub enum Goal { Boot { recover: bool }, Desk(u8), Window(WinKey), Foreign(WinKey),
                Carry(u8), UndoCarry { win: WinKey, to: u8 }, Verify, ShowAll }
pub enum FocusTo { Win(WinKey), Shell, Keep }
pub struct Plan { active: u8, set: Vec<(WinKey, u8)>, show: Vec<WinKey>,
                  focus: FocusTo, hide: Vec<WinKey>, note: String }
impl Desks { pub fn plan(&mut self, goal: Goal, wins: &[WinInfo], fg: Option<WinKey>) -> Result<Plan, Skip>; }
```

`WinInfo` gana `desk: Option<u8>` (prop `wm.desk`), `marked`, `concealed` (cloak ≠ 0 o minimizada) y `minimized`.

**Reglas de `plan`:**
- **Adopción** (en todo `Event::Windows`): ventana desconocida → `desk = activo` + `SetDesks`. La prop solo se cree en `Boot { recover: true }`. Las que no están en el Scan se podan (también de `placed`, `centers`, `sizing`, `limits`). Fuera de `scope` → `sticky`.
- **show** = marcadas con `desk == activo` o `sticky`.
- **hide** = `desk != activo`, no sticky, no en vuelo (antes, `Land`) y no (`marked && concealed`).
- **focus** = `last_focus[n]` si sigue en n y no está minimizada; si no, la primera en Z de n no minimizada; si no, `Shell`.
- **`Carry(n)`:** la del frente (dueño raíz) pasa a n; `last_focus[n] = ella`; foco a la siguiente del activo; entra al diario. No se sigue a la ventana.
- **`Desk(n)` con n activo:** solo reconcilia ("reparar" gratis).

| Caso | Diseño | Estado |
|---|---|---|
| Elevada, no marcable | No se detecta de antemano. Si `SetDesks` o `Hide` fallan en esa ventana → `sticky` + log | Robusto a los dos resultados (manual pendiente) |
| Minimizada | Se esconde como cualquiera con cloak. Nunca se elige como foco (`activate` la restauraría). Con minimize, las ya minimizadas se saltean sin marcar | Verificado |
| Diálogo con dueño | No se gestiona. Todo HWND de `FOREGROUND` (y el `fg` del Scan) se traduce al dueño raíz (`GA_ROOTOWNER`) | Verificado: el cloak lo arrastra |
| Pantalla completa, UWP | Sin caso especial | Verificado con títere y Configuración; video real pendiente |

**Diario:** `Entry` gana `desk: Option<u8>` (`Some(desde)` = "llevar"). Deshacerla lanza `Goal::UndoCarry`; si la ventana ya no existe, pasa a la anterior en la misma tecla.

## 3. La transacción

`World` gana `desks`, `sync: Option<Sync { goal, phase, prev_active, key_us }>`, `queued: Option<Goal>` (gana el último; `Verify` no pisa un pedido del usuario) y `verify_due: Option<u64>` (entra en `deadline()`). El pegamento va en `world_desks.rs` como `impl World`.

- **Efectos nuevos:** `SetDesks { items }`, `Show { req, wins }`, `Hide { req, wins }`, `FocusShell { req }`, `RestoreAll { forget }`.
- **Eventos nuevos:** `Boot { recover }`, `Foreground(WinKey)`, `Uncloaked(WinKey)`, `ShellRestarted`, `Shown { req, failed, timed_out }`, `Hidden { .. }`, `DesksSet { failed }`, `Command::ShowAll`.

**"Ir a N":** Scan → `plan` (`active = N`, se guarda `prev_active`) → `Show` en lote (carril `shell`) → `Focus` o `FocusShell` (carril `focus`) → `Hide` en lote (carril `shell`) → log con ms desde la tecla → corre `queued`. Nunca se ve el fondo: el origen se esconde al final y el foco ya salió de ahí.

**Costo** (0,9 ms por mostrar, 1,2 ms por esconder): 5 + 5 ventanas ≈ 15 ms; 10 + 10 ≈ 25 ms. Si la meta de p95 < 30 ms queda justa, Foco y Esconder pueden ir en paralelo.

| Falla | Respuesta (una sola por causa) |
|---|---|
| `Show` falla entero (plazo, o todas con error: Explorer colgado o reiniciando) | `active = prev_active`, no se esconde nada, log |
| `Focus` falla | Un `FocusShell` y se sigue: nunca teclear sobre una ventana invisible |
| `Hide` vence por plazo | `Ops::degrade` lo hace con minimize. El modelo no reintenta: el próximo Sync lo ve |
| `Hide` falla en una ventana | `sticky` + log; "Mostrar todas" lo limpia |
| Explorer reinicia a mitad | Errores COM, igual que arriba; `TaskbarCreated` → `Goal::Verify` |

## 4. Pasos (un commit cada uno)

### 4a.1 Plataforma, sin cambio de comportamiento
- **`hide.rs`:** `PROP_DESK`, `set_desk`, `desk_of`, `forget_desks()`. `hide`: marcada con el mismo método **y** `looks_hidden` → nada; marcada pero visible → reaplicar; con minimize y ya minimizada sin marca → `Ok` sin marcar. `hide_many` / `show_many` con resultado por ventana, verificación de pid y confirmación de `DWMWA_CLOAKED` (sondeo ≤ 50 ms para todo el lote).
- **`windows_list.rs`:** `WindowInfo` += `desk`, `cloaked`, `minimized`; `root_owner()`; `shell_window()`. Las marcadas no se descartan por título vacío ni por no poder leer el proceso.
- **`ops.rs`:** `Op::SetDesks` (win), `ShowMany` y `HideMany { method }` (shell), `FocusShell` (focus; `activate(Progman)`), `RestoreAll { forget_desks }`, `CloakProbe`; `Output::Batch`. Plazo de lote: 400 ms.
- **Pruebas (C):** chequeo "lotes" con 3 títeres.

### 4a.2 Ir y llevar
- `desks.rs`, `world_desks.rs`, `world.rs` (dígitos, adopción en `Windows`, `Entry.desk`), `brain.rs` (`Wait::{SetDesks, Show, Hide}`, `hide_method`, `cloak_proven`, `CloakProbe` al arrancar y minimize si falla, `WM_SCOPE`, `focus()` devuelve el dueño raíz), `core.rs` (al terminar `RestoreAtStart` → `Event::Boot { recover }`).
- **Pruebas (P):** `world_desk_tests.rs` con plataforma simulada; invariantes 1–3; deshacer(llevar) = identidad; 10 000 secuencias al azar que terminan sin nada en vuelo y con visible ⇔ `desk == activo`.
- **Pruebas (C), "escritorios":** `Caps + Alt + 2`, `Caps + 2` / `Caps + 1`, 50 alternancias con p95, `Caps + Alt + Z`. Se saltea si hay otro `wm-ui`.

### 4b.1 `Caps + letra` y mapa
- `go_to_app`: ventana de otro escritorio → `Goal::Window`, reutilizando ese Scan.
- `mapview.rs`: fila 1–4 (activo en accent con "N ventanas"; con ventanas, dígito en `text` y cantidad en `text-2`; vacío en `text-3`); capa de ventana "llevar" (el activo, `Dim`); pies con "1–4 = escritorios" / "1–4 = llevar". `overlay_text.rs`: una fila más. `ui.rs`: `fade_in` solo si no estaba visible; al terminar un Sync con el mapa visible se reemite `ShowMap`.

### 4b.2 WinEvents
- `winevents.rs`: en el hilo del brain, `SetWinEventHook` fuera de contexto con `SKIPOWNPROCESS` para `EVENT_SYSTEM_FOREGROUND` y `EVENT_OBJECT_UNCLOAKED`; filtra `idObject == 0`.
- **`Foreground(k)`:** conocida y del activo, o sticky → `last_focus`/`last_fg`. Desconocida → Scan de adopción. De otro escritorio → `Goal::Foreign(k)`: si el foco actual ya no es k → solo `Verify`; si `last_fg` sigue viva y sin minimizar → **salto** (`active = desk(k)`, foco `Keep`); si `last_fg` desapareció o se minimizó (Windows activó la siguiente en Z) → **rebote**: no se cambia de escritorio, foco a la mejor del activo o al shell, y k se re-esconde.
- **`Uncloaked(k)`:** del activo, sticky, desconocida o en vuelo → nada. Si no → `verify_due = ahora + 150 ms` → `Verify`. Re-esconder suma `strikes`; con 2 → `sticky` + log (R4).

### 4c Recuperación
- **Salida limpia:** `RestoreAll { forget_desks: true }`. Tras un pánico, sin olvidar y código 3.
- **Guardián:** el `restore_all` al morir el hijo sigue igual (`wm.desk` sobrevive). `forget_desks()` al arrancar y en toda rama que no relanza.
- **D4:** `WM_RELAUNCH ≥ 1` → `Boot { recover: true }`: activo = `desk(fg)`, si no el de la primera en Z con prop, si no 1; `plan` re-esconde. Tras una caída se ven todas ~300–500 ms (aceptado: "todo visible mientras no hay core").
- **`TaskbarCreated`** → `Goal::Verify`.
- **"Mostrar todas las ventanas"** en la bandeja: todas al activo, se limpian sticky y strikes, `RestoreAll` + `SetDesks`. `wm restore` con un core vivo manda esa orden.
- **Pausar** no toca escritorios.

### 4d Monitores (mínimo)
En `show_many`, antes de destapar: si no está minimizada y no es alcanzable → `screens::rescue`. Lo demás (`display_watch` + `StormTracker` en el brain) queda para después: el mapa y el fantasma ya se rearman solos.

## 5. Respaldo minimize

El modelo no cambia. En la plataforma: las ya minimizadas ni se marcan ni se restauran; el clic en la barra o Alt+Tab restaura → `FOREGROUND` → salto; no hay rebotes; el orden Z por escritorio no se conserva; sin animaciones.

## 6. Verificaciones (hito 4.0) → decisión

| Verificación | Resultado | Efecto |
|---|---|---|
| Ruido en reposo | 0 eventos en 2 s | Escuchar `FOREGROUND` y `UNCLOAKED` no cuesta en reposo |
| Propios `CLOAKED`/`UNCLOAKED` | Llegan (1 y 1) | Filtro por `desk == activo`; `CLOAKED` no se registra |
| Foco a una escondida | Windows la destapa sola, llega `FOREGROUND`; la marca queda | El salto la desmarca; el rebote la re-esconde |
| Minimizada, diálogo con dueño, pantalla completa, UWP, `Progman` | OK | Sin casos especiales; `FocusShell` = `Progman` |
| Clic real en la barra, Alt+Tab, elevada | Pendiente (`wm desk-check manual`) | El diseño sirve en los dos casos |
| Video real a pantalla completa | Pendiente | Si da problemas: `Desk`/`Carry` no hacen nada con pantalla completa al frente |
| (viii) Al minimizar o cerrar la del frente con una escondida más arriba en Z, ¿a cuál activa Windows? | Pendiente (`desk-check`) | Si activa la escondida, el rebote lo cubre |
| ¿La espera del brain despierta con un WinEvent fuera de contexto? | Pendiente | Si no: hilo `wm-events` con canal, como la bandeja |

## 7. Orden y recortes

4a.1 → 4a.2 → 4b.1 → 4b.2 → 4c → 4d. **Imprescindible para la sesión 1:** 4a, 4b.1, `FOREGROUND` de 4b.2 y todo 4c. **Recortable:** `UNCLOAKED` (cada cambio ya reconcilia) y el `StormTracker`. **Opcional:** sufijo de `WM_INSTANCE` en los nombres de las props, para que el `restore_all` del `ci-probe` no destape lo que el core real tiene en otros escritorios.
