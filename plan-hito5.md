# Hito 5: mouse, diseño sobre el código

Diseño de Fable 5.2 (16/11/2026) sobre el código real después del hito 4. Completa la fila del hito 5 de `plan-core.md` §7; cada paso cerrado se anota ahí.

**Medido en esta PC** (2 000 ciclos, en `TIME_CRITICAL`): poner `WH_MOUSE_LL` p50 0,8 µs, p99 1,2 µs, máx 33 µs; quitarlo p50 0,9 µs, p99 1,3 µs, máx 26 µs. Instalarlo en cada pulsación de `Caps` entra sobrado en el presupuesto del hook (p99.9 < 200 µs).

## Decisiones de fondo

1. **El hook de mouse sigue a "`Caps` abajo"**, en cualquier capa (con o sin Alt). Vive mientras `hyper || algún botón tragado sigue abajo`.
2. **El movimiento no pasa por el hook ni por el brain.** El hook solo traga y avisa botón abajo/arriba. Un carril `wm-drag` lee `GetCursorPos` una vez por cuadro mientras dura el arrastre. El movimiento y la rueda pasan directo (`CallNextHookEx`).
3. **El arrastre sigue hasta soltar el botón, aunque se suelte `Caps` antes** (como AltSnap). El "arriba" del botón tragado se traga igual: un `WM_RBUTTONUP` huérfano abre el menú contextual.
4. **Con `Caps` abajo, clic izquierdo y derecho se tragan siempre.** Qué ventana es y si se puede lo decide `World`, no el hook.
5. **Z deshace el arrastre entero** (cambia `plan-core.md` §3.2, que excluía el mouse): "Deshacer" de la bandeja le sirve a quien usa mouse.
6. **Redimensionar en vivo**, desde la esquina más cercana (cuadrantes 2×2), con contrapresión. Sin rectángulo de vista previa.
7. **Recortado: arrastrar una ventana al indicador de la bandeja** (blanco de 16 px, en Windows 11 el ícono nace en el desborde, un ícono no da cuatro destinos).

## 5a. Hook temporal y máquina `HyperMouse`

- **`wm-model/src/mouse.rs`** (puro): `Button {Left, Right}`, `MouseEvent {Down, Up}`, `HyperMouse::on_event(e, hyper) -> MouseDecision { pass, signal }`, `held()`, `reset()`. `Down` con hyper se traga (avisa solo si no hay otro tragado); `Up` de un tragado se traga, también después de soltar `Caps`; todo lo demás pasa.
- **`keyboard.rs`:** `HyperKeyboard::mark_used()` (como una tecla nueva): un toque de `Caps` con clic no cambia Bloq Mayús.
- **`input.rs`:** `mouse_proc` (solo L/R abajo/arriba; pausa o brain muerto → pasa); `KIND_POINTER`, `InputEvent::Pointer { button, down, x, y, time_ms }`. **Poner** en `hook_proc` cuando la capa sale de `Off` (quitar y poner si no hay botón retenido: un hook que Windows quitó dura una pulsación). **Quitar** siempre por mensaje al `SINK` (`sync_mouse_hook`, idempotente: `want = hyper || held`), nunca dentro de su propio callback. Todas las rutas de reinicio pasan por `reset_machines()`. Contadores `mouse_installs` / `mouse_removes`.
- **Tope:** `World.pointer` vence a los 60 s → `ResetKeyboard`.
- **P:** 10 000 secuencias: el sistema nunca ve un arriba sin su abajo; las señales alternan; al final `!held()`. Un clic dentro de una pulsación de `Caps` nunca cambia Bloq Mayús.

## 5b. Arrastrar para mover

Flujo: `Pointer` abajo → `Effect::InspectAt { x, y }` (carril `win`: `WindowFromPoint` → `GA_ROOT`; rechaza nuestras, con cloak, de herramienta, `Ghost`, sin título que no sean de usuario) → `Event::Grabbed` → `Effect::Raise` + `Effect::DragStart` → al soltar `Effect::DragStop` → `Event::DragDone { moved }`.

- Pantalla completa: nada. Antes: `ground()` y fuera de `jobs`, `placed`, `centers`, `sizing`. `Entry` antes de empezar; se anota en `DragDone` si se movió. Al terminar, rescate si el título quedó fuera de pantalla.
- **Maximizada:** se restaura como al arrastrar el título, bajo el cursor en la misma proporción; Z la vuelve a maximizar.
- **`wm-model/src/drag.rs`:** `Session` (umbral `SM_CXDRAG`, `at(p) -> Option<Rect>`).
- **Carril `wm-drag`** (como `anim`): por cuadro, `GetCursorPos` → `SetWindowPos` asíncrono con `SWP_NOSIZE`. **Contrapresión:** un pedido pendiente hasta que la ventana cambió, llegó o pasaron 100 ms. `SWP_NOSIZE` deja que la app se reescale sola al cruzar de DPI.
- Soltar `Caps` a mitad no corta el arrastre.

## 5c. Clic derecho + arrastre: redimensionar

Esquina del cuadrante del cursor; la opuesta queda fija. Límites con lo aprendido (`World.limits`) y aprendizaje en vivo (`learn(asked, got)`), devuelto a `limits`. Maximizada o sin `WS_THICKFRAME`: nada (el clic derecho se traga entero).

## Interacción con lo existente

| Caso | Diseño |
|---|---|
| Mapa | Un clic lo suprime; visible → se oculta |
| Bloq Mayús | `mark_used()` |
| Teclas durante un arrastre | Se ignoran (Z lo deshace después) |
| Vuelos | `ground()` antes de empezar |
| Escritorios | Las de otro escritorio tienen cloak y se rechazan; con un cambio en curso, el clic se ignora |
| Pausa | El hook no se pone |
| Rueda, doble clic, botón medio | Pasan |
| Snap de Windows | No se dispara con `SetWindowPos` (bordes propios: con las zonas, v2) |

Para descubrirlo: una línea en el pie del mapa y en el banner.

## 5d. Sonda: chequeo "arrastre"

`session::send_mouse` (`SendInput` absoluto). Plan A: arrastre izquierdo de (120, 80) sobre un títere, Bloq Mayús igual, Z lo devuelve; derecho desde abajo a la derecha crece con la esquina de arriba a la izquierda fija; `mouse_installs == mouse_removes`. Plan B si el mouse inyectado no llega (architecture.md §13.11): `WM_POINTER_KEYS=1`, `Caps + F8 / F7 / F9` = izquierdo abajo / derecho abajo / arriba, con `SetCursorPos`.

## 5e. Bandeja: indicador de escritorios

- Número del escritorio activo en el ícono (4 íconos en caché); tooltip "wm: escritorio 2".
- Menú con los escritorios arriba ("Escritorio 1 · 3 ventanas ✓ … Escritorio 4 · vacío") → ir.
- **Clic izquierdo abre el mismo menú**, no los ajustes ("Ajustes…" queda como ítem).
- Antes de la sesión: fijar el ícono en la barra de su PC.

## 5f. "Llevar" desde el menú (recortable)

La bandeja recuerda la última ventana de usuario al frente; submenú "Llevar «Brave» a ▸ 1–4" → `Goal::Send { win, to }` (como `Carry` con esa ventana; entra al diario).

## Orden

5a → 5b → 5c → 5d → 5e → 5f → documentación. **Imprescindible para la sesión 1:** 5a–5c y 5e. **Recortable:** 5f, reescalar por DPI durante el arrastre y el rescate al soltar.
