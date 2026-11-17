# Plan v1.2 (0.1.7): las cosas chicas

Diseño de Fable del 17/11/2026, a pedido del usuario ("seguí con las cosas más chicas que quedaron anotadas"), antes de la v2 con zonas. Se escribió leyendo el código, sin compilar ni medir: el paso 0 confirma lo que quedó a verificar.

## 1. Alcance

| # | Ítem | v1.2 | Por qué |
|---|---|---|---|
| 7 | Registro de uso | Sí, primero | Las 1–2 semanas de uso real son ahora (`keyboard-design.md` §12.3); chico y sin riesgo |
| 3 | Aviso en los ajustes (app en dos letras) | Sí | Solo toca `wm-settings` |
| 2 | "Abrir siempre así" desde la bandeja | Sí | Reusa `locate`, `OpenRule`, `update_bindings` y `wm-watch` |
| 1 | Escritorio al abrir | Sí, siguiendo a la ventana | Al seguirla no existe "abrí la app y desapareció" |
| 6 | Enseñar en el momento | Parcial | 6a (Alt+Tab o la barra → "La próxima vez: Caps + B") sí; 6b (Snap con el mouse) pasa a v2: pide un hook nuevo y en v2 el arrastre a zonas ya es nuestro |
| 4 | Respaldo de glifos | Sí, último, si la medición da | Aislado en `overlay_text.rs`; costo cero con texto latino |
| 5 | Pausarse sola con pantalla completa | No | Ver §7 |
| 8 | Documentos desactualizados | Sí | Texto |

## 2. Registro de uso (ítem 7)

- **Opción:** `registro_de_uso = sí|no` en `settings.txt`, apagada por defecto (`Options.usage_log`). Interruptor en la pestaña Opciones; `wm uso` imprime el archivo ordenado por veces.
- **Archivo:** `%LOCALAPPDATA%\wm\uso.txt`, contadores por día, **sin títulos ni nombres de app**:
  ```
  # Registro de uso de wm. Solo en esta PC; no se envía a ningún lado.
  # día | qué | veces
  2026-11-18 | Caps + B | 14
  2026-11-18 | Caps + Alt + A | 3
  2026-11-18 | Caps + Alt + D (no hizo nada) | 1
  2026-11-18 | Mapa | 5
  2026-11-18 | A mano: app con letra B | 9
  ```
- **Clave de cada línea:** `crates/wm-model/src/usage.rs`, con el patrón de `notices.rs`. `key_for(line)`: las líneas `Caps + …` hasta ` → ` o `:`, con " (no hizo nada)" si `notices::notice_for` da algo o dice `FALLÓ`; casos fijos: mapa visible, regla al abrir, atajos en pausa, mostrar todas, abrir siempre así, "A mano … tiene la B". `Usage` con `parse`, `add`, `format` y `prune` (60 días).
- **Dónde se cuenta:** en el hilo `wm-log` (`brain.rs`), por donde ya pasan todas las líneas: el canal pasa a `Msg { Line, Usage(bool), Teach(String) }`. Escribe (temporal + rename) si hay cambios y pasaron 10 minutos, al recibir una línea; también al apagar la opción y al cerrar. Sin temporizador: en reposo no despierta.
- **Bordes:** una caída pierde hasta 10 minutos; líneas malas se ignoran; disco lleno no muestra error.

## 3. Misma app en dos letras (ítem 3)

- `assign(st, app) -> Note` en `settings/src/main.rs`, usada por `on_picker_choose` y `on_picker_enter` (Enter hoy no borra la regla al cambiar de app: se arregla acá).
- Si la app ya estaba en otra letra: **se muda** (la otra queda libre; regla, escritorio y `extras` se van con ella). Si la app de la letra cambia sin mudanza: se borran regla, escritorio y `extras`.
- Apps abiertas sin letra cuya inicial es la letra elegida (el criterio de `apps::resolve`): se avisa que `Caps` + letra ya no va a ellas.
- Dos textos chicos bajo "Al abrir"; se borran al cambiar de letra o buscar. Cancelar no guarda nada.
  - `{} estaba en la {}. Ahora está solo en la {}.`
  - `Caps + {} ya no va a ir a: {} (abiertas ahora). Si las usás, dales otra letra.`

## 4. "Abrir siempre así" (ítem 2)

- **Menú de la bandeja** (`tray.rs`): `Lane.last_user` (la última ventana de usuario al frente que no es nuestra); al abrir el menú, `front_app()` busca su letra en `apps.txt`.
  - Con letra: `Abrir Brave siempre así` (o `…  ·  ahora: mitad derecha`).
  - Sin letra: en gris, `Abrir AutoCAD siempre así (dale una letra en Ajustes)`.
  - Sin ventana de usuario: no aparece.
  - Clic → `Command::OpenLikeThis { win, letter }`; `Order::OpenLikeThis` (`wm abrir-asi`) para `ci-probe`.
- **Modelo:** `Inspect` de la ventana → `looks_maximized`, `locate` (respeta `placed` y el espacio) y `rule_like(frame, maximized, placement, work)`: maximizada, mitad o esquina, o centrada (30–100 %, tolerancia 1 % del ancho). Resultados: igual a la que tiene → "Brave ya se abre así"; libre, minimizada o pantalla completa → "Acomodala primero: mitad, esquina, maximizada o centrada"; cerrada → "Esa ventana ya no está"; si no, `Effect::SaveRule`. La confirmación ("Brave se va a abrir siempre en la mitad derecha") sale cuando vuelve `Event::Bindings`, así prueba el circuito entero. Estos avisos salen aunque los avisos estén apagados (responden a un clic).
- **Guardar:** `apps::set_open_rule(text, letter, open)`; el brain escribe en un hilo corto (`apps.txt.wm.tmp` + rename); si falla, globo en la bandeja.
- **Ajustes abiertos a la vez:** al guardar, releer el archivo y respetar lo que cambió afuera en las letras que no se tocaron.
- No mueve ventanas: no hay Z. Se saca en Ajustes → "Al abrir: No hacer nada". **No guarda el escritorio.**

## 5. Escritorio al abrir (ítem 1)

- **Formato:** `b = brave | | Brave | al abrir: derecha | escritorio: 2` (la v0.1.6 ya conserva el campo como `extra`). `AppBinding.desk: Option<u8>` (1–4); `AppRule { open, desk }`.
- **Comportamiento:** las mismas condiciones que la regla de hoy (ventana nueva, la primera principal de la app, al frente, sin pausa). wm la lleva al escritorio N **y va con ella**, con el aviso "Brave se abre en el escritorio 2". Si ya estás en el N, nada.
- `desks.rs`: `Goal::Bring { win, to }` y `Goal::BringBack`. Cola con prioridad: tecla del usuario > `Bring` > `Verify`. Se aplica en la primera observación (+100 ms); `apply_rule` se separa en `apply_open` y `apply_desk`.
- **Z:** con regla combinada son dos entradas: la primera devuelve la posición, la segunda vuelve con la ventana al escritorio de antes (`Entry.went`).
- **Ajustes:** fila "En el escritorio" (En el que estés / Escritorio 1–4) bajo "Al abrir".
- **Bordes:** ventanas presentes al arrancar wm, segunda ventana de la app y ventanas elevadas no se llevan; una tecla de escritorio del usuario gana.

## 6. Sugerir atajos (ítem 6a)

- **Detección:** si la ventana que pasa al frente es del shell (`Shell_TrayWnd`, `XamlExplorerHostIslandWindow`, `MultitaskingViewFrame`, `TaskSwitcherWnd`, `ForegroundStaging`…; a confirmar en el paso 0) y en menos de 5 s llega una ventana de usuario distinta → `Event::Chosen`.
- **Modelo:** la letra de cada ventana conocida; se ignora con pausa, `Caps` abajo, transacción o foco nuestro reciente. Log `A mano (Alt+Tab o la barra) → brave: tiene la B.` y aviso "La próxima vez: Caps + B".
- **Topes** (`teach.rs`, puro): recién al 3.er cambio a mano en la sesión; como mucho uno cada 10 minutos; nunca más tras 5 usos de `Caps` + esa letra o tras 5 avisos de esa letra. Persistencia en `%LOCALAPPDATA%\wm\aprendizaje.txt`.
- **Opción:** `sugerir_atajos = sí|no`, prendida por defecto ("Sugerir atajos: cuando vas a una app con Alt+Tab o la barra de tareas, te recuerda su letra"). `ci-probe` la apaga.

## 7. Pantalla completa: no se pausa sola (ítem 5)

Hoy, con una app a pantalla completa, un toque de `Caps` es Bloq Mayús, el mapa no aparece, `Caps` + letra o número funcionan (sirven para salir) y `Caps` + Alt no hace nada. Para esta usuaria, pantalla completa es AutoCAD con Ctrl+0, Brave con F11, un video o una presentación: justo donde `Caps` + letra es lo que quiere. Una pausa automática sería "`Caps` dejó de andar" sin explicación, y `SHQueryUserNotificationState` no distingue juegos de F11 ni avisa cuando cambia. Si aparece el caso (juegos, escritorio remoto, máquinas virtuales), lo correcto es "Pausar atajos en esta app" desde la bandeja.

## 8. Respaldo de glifos (ítem 4)

- La lista ya omite lo que Geist no tiene; el mapa y los avisos llaman a `text_geometry` directo y ahí sí salen cuadraditos.
- Caras por archivo desde `%WINDIR%\Fonts`, abiertas recién cuando Geist no tiene un carácter: `segoeui.ttf`, `seguisym.ttf`, `seguiemj.ttf`, `msyh.ttc`, `malgun.ttf`. Se omiten controles, ZWJ, selectores y las escrituras que necesitan shaping o van de derecha a izquierda.
- **Compuerta** (`wm overlay-demo fuentes`): camino latino igual (±0,1 ms), respaldo ≤ 3 ms por armado, ≤ 0,5 MB más.

## 9. Orden

0. Observar sin cambiar nada: clase de la ventana al frente con Alt+Tab, barra, Win+Tab y clic; línea de base de fuentes.
1. Documentos viejos.
2. Ajustes: misma app en dos letras.
3. Registro de uso.
4. "Abrir siempre así".
5. Escritorio al abrir.
6. Sugerir atajos.
7. Respaldo de glifos.
8. Documentos de v1.2, 0.1.7, release.

Corte posible: publicar tras el paso 4 y dejar 5–7 para la 0.1.8.

## 10. Decisiones del usuario

Del 17/11/2026, todas con la recomendación de Fable:
- Escritorio al abrir: wm **va con la ventana** al escritorio N.
- Misma app en dos letras: **se muda y avisa**; también avisa qué apps abiertas pierden la letra.
- Sugerir atajos: **prendido** por defecto, solo para apps (Snap con el mouse, en v2).
- "Abrir siempre así": guarda cómo está la ventana al frente, sin el escritorio; app sin letra = ítem en gris.
- Pantalla completa: **no** se pausa sola.
- Registro de uso: **apagado** por defecto; solo "qué atajo, cuántas veces por día".
