# Funciones escondidas en la primera versión

Qué se escondió de la interfaz antes de que la usuaria de prueba use `wm`, por qué, con qué requisito se relaciona y qué falta para volver a mostrarlo. Escrito el 16/11/2026, a pedido del usuario, **para que no se olvide**: cada ítem es trabajo pendiente, no algo descartado.

> **Actualización (16/11/2026, v0.1.1):** ya existe el archivo de opciones (`%APPDATA%\wm\settings.txt`, `wm_model::options`, recargado solo por `wm-watch`) y **volvieron a los ajustes las tres opciones de §2–§4**, que ahora funcionan: borde de foco (`DWMWA_BORDER_COLOR`, solo Windows 11), el puntero sigue al foco (al ir a una app con `Caps` + letra o al cambiar de escritorio) y animar ventanas. Se sumó una cuarta, **"Avisos en pantalla"** (plan-core.md §10). **Sigue escondido solo el perfil (§1).** Lo que queda pendiente de cada una está anotado abajo.

**Regla:** en la primera versión no se muestra nada que no haga lo que dice. Una persona no técnica que toca un interruptor y no pasa nada concluye que el programa está roto (`plan-core.md` §10: "que no pase nada parece una falla"). Por eso estas opciones se esconden, no se dejan "para probar".

**Cómo verlas igual:** `wm-settings.exe --show-unfinished`. Están detrás de la propiedad `show-unfinished` de `settings/ui/settings.slint` (por defecto `false`); el código y las traducciones siguen ahí. Para volver a mostrar una sola, sacarla del bloque `if root.show-unfinished`.

La versión nativa de los ajustes (`wm settings`, `wm-platform::native_settings`) es la otra mitad del spike 8 y no la abre la bandeja: no se tocó.

---

## 1. Perfil "Apps primero / Ventanas primero"

- **Qué era:** el control segmentado "¿Qué querés más a mano?" arriba de la lista de letras.
- **Por qué se esconde:** no se guarda ni cambia nada. El core tiene fijo el mapa oficial "Apps primero": `Caps` + letra va a la app y `Caps` + Alt acomoda la ventana (`wm_model::keyboard`, `world.rs`).
- **Requisitos:**
  - `keyboard-design.md` §6 y §14.2: "Apps primero" es el mapa oficial (opción B);
  - `keyboard-design.md` §8.1: "Ventanas primero" (opción A) es un perfil opcional, el mismo mapa con las capas intercambiadas, y se elige en la bienvenida (que también está fuera de v1, línea de alcance de `plan-core.md`);
  - `plan-core.md` §7, "Antes de la sesión U": ocultar el selector; el perfil va después de v1.
- **Para hacerlo:**
  - guardar el perfil (por ejemplo, una línea en un `settings.txt` junto a `apps.txt`, recargado por `wm-watch` como `apps.txt`);
  - que el core intercambie qué capa lleva el pulgar (`HyperKeyboard` ya distingue `thumb`; el cambio está en cómo `World::action` interpreta `Letter { thumb }`);
  - que el mapa (`mapview.rs`) muestre las capas en el orden del perfil, y su pie diga lo correcto;
  - la pregunta de la bienvenida (`keyboard-design.md` §8.1: *"¿Qué hacés más seguido: cambiar de app o acomodar ventanas?"*).
- **Relacionado:** los perfiles de teclas de dirección (WASD, flechas, Vim, `keyboard-design.md` §8.2) tampoco existen todavía (`plan-core.md` §10, "Teclado y configuración").

## 2. "Borde de foco"

- **Qué era:** "Una línea de 2 px alrededor de la ventana activa", prendido.
- **Por qué se esconde:** no existe. Ninguna parte de `wm` dibuja un borde.
- **Requisitos:**
  - `findings.md` §17: *"El borde de foco es una función, no un adorno"* (los tutoriales de AeroSpace instalan JankyBorders para saber qué ventana tiene el foco);
  - `keyboard-design.md` §14.7 y `design.md` §5: prendido por defecto, 2 px en `accent` (`design.md`: "nunca otro color");
  - `architecture.md` §3 y §7.3: overlay con `CompositionSpriteShape`; `plan-core.md` §10, "Después de v1": con `DWMWA_BORDER_COLOR` cuesta poco.
- **v0.1.2: hecho lo completo** (`focus_border.rs`): una ventana propia, transparente a los clics, con la línea de 2 px en `accent` justo afuera del marco de la ventana activa y justo encima de ella en el orden Z; la sigue con WinEvents acotados a su proceso (`LOCATIONCHANGE` y cloak), ~5 ms de retraso; se esconde con la ventana minimizada, maximizada, a pantalla completa o con cloak. Se ve también en Terminal y Claude. Reemplaza al de `DWMWA_BORDER_COLOR` de la v0.1.1, que estaba **limitado (prueba del usuario, 16/11/2026):** el borde se ve en Brave y Steam, pero **no en Windows Terminal ni en Claude**: las apps que dibujan su propio marco (Electron sin marco, Terminal) ignoran `DWMWA_BORDER_COLOR`. Para esas hace falta lo completo. **Queda:**
  - ~~lo barato y documentado: `DwmSetWindowAttribute(DWMWA_BORDER_COLOR)` sobre la ventana al frente y devolverle el color por defecto a la anterior (solo Windows 11; el `EVENT_SYSTEM_FOREGROUND` ya llega al brain desde el hito 4b.2)~~;
  - lo completo: un overlay propio que siga a la ventana (necesita `EVENT_OBJECT_LOCATIONCHANGE` acotado al proceso de esa ventana, `architecture.md` §4.3);
  - guardar la opción y respetarla en el core;
  - probar con ventanas sin borde, maximizadas y a pantalla completa.

## 3. "El puntero sigue al foco"

- **Qué era:** "Cuando cambiás de app, el puntero va con ella", apagado.
- **Por qué se esconde:** no existe.
- **Requisitos:** `findings.md` §17 ("el puntero sigue al foco como opción", como `mouse_follows_focus` de yabai o `move-mouse window-lazy-center` de AeroSpace); `keyboard-design.md` §10.1 y §14.7; `plan-core.md` §10.
- **Para hacerlo:**
  - después de un `Focus` exitoso de `Caps` + letra (o de un cambio de escritorio), mover el puntero al centro de la ventana con `SetCursorPos`, salvo que ya esté adentro ("lazy");
  - un efecto nuevo en `World` (`Effect::MovePointer`) para que sea probable con la plataforma simulada;
  - guardar la opción (apagada por defecto).

## 4. "Animar ventanas"

- **Qué era:** "Movimientos cortos, sin rebote", prendido.
- **Por qué se esconde:** la animación existe (hito 1e: vuelos con fantasma y cambio de tamaño), pero **el interruptor no hace nada**. Hoy solo se apaga con la variable `WM_ANIM=0` o con "Reducir animaciones" de Windows (que `wm` respeta: `windows_list::reduced_motion`).
- **Requisitos:**
  - `findings.md` §7, pedido 30 ("animaciones en cero");
  - `plan-core.md` §11: hecho, falta el interruptor en los ajustes;
  - `plan-core.md` D6: las animaciones son opcionales.
- **Para hacerlo (es el más barato de los cuatro):**
  - guardar la opción en el mismo archivo que el perfil;
  - que el brain la lea al arrancar y al recargar, y mande `Event::FlyEnabled(false)` (ya existe);
  - mostrar solo este interruptor (sacarlo del bloque `if root.show-unfinished`).

---

## Pendiente común

Los cuatro necesitan lo mismo: **un archivo de opciones** que escriban los ajustes y lea el core (hoy los ajustes solo escriben `apps.txt`). Conviene hacerlo una sola vez, con el mismo cuidado que `apps.txt`:
- se escribe en un temporal que se renombra encima;
- se recarga solo con `wm-watch`;
- una línea por opción, legible a mano.

Al terminarlo, cada opción se muestra apenas funcione: no hace falta esperar a tener las cuatro.
