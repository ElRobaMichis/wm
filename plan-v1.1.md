# v1.1: reglas por app (diseño de Fable 5.2, 17/11/2026)

Sobre lo pedido en `findings.md` §7 (⭐4 reglas por app, 13 ventanas nuevas en el monitor del puntero, 26 espacios entre ventanas) y la hoja de ruta de `plan-core.md` §11.

## 1. Alcance

- **(a) "Al abrir" por app: sí, es el núcleo.** Sale casi gratis sobre `go()`: normaliza, verifica en pasadas, ancla el tamaño mínimo y no pelea.
- **(d) Espacios entre ventanas: sí, último paso, apagado por defecto** (con 0 no cambia nada).
- **(b) Monitor del puntero: sí, opcional,** apagado y oculto con un solo monitor.
- **Después:** (c) "no tapar a otras" (v2, con zonas: con ventanas maximizadas nunca hay lugar libre y mover sin regla es pelear); (e) "escritorio N al abrir" (v1.2: "abrí la app y desapareció" confunde); recordar la última posición y monitor fijo por app (v2.x); "abrir siempre así" desde la bandeja.

## 2. Comportamiento de "al abrir"

A qué ventana se aplica, **una sola vez**: nueva (no estaba al arrancar `wm`), al frente cuando se la ve por primera vez, **principal** (con borde redimensionable y botón de maximizar: descarta pantallas de carga y diálogos), de una app con regla y sin otra ventana principal abierta (no se mueven la segunda ventana de Brave, los popups ni las pestañas arrancadas), no minimizada, no a pantalla completa, no en otro escritorio, sin pausa ni tormenta de pantallas.

**Espera a que la app termine de ubicarse:** se mira a +100, +300, +700 y +1500 ms y se aplica cuando marco y estado repiten; si nunca se queda quieta, no se mueve (queda en el log). **Se cancela** si hay un botón del mouse abajo (arrastre o pestaña arrancada), si el usuario la movió con `Caps` o la arrastró, si se destapó (escritorio virtual nativo) o si se cerró.

**Aplicar:** en el monitor donde la abrió la app (o el del puntero con (b)); mitad/esquina con `go()` (queda en `placed`: A/D cuentan desde ahí y 4d la reencaja), maximizada o centrada; **sin animación**; **Z la devuelve** adonde la abrió la app. **No pelear:** una aplicación por ventana; si la app se reubica después, gana la app.

## 3. Detección: sin hooks nuevos

`EVENT_SYSTEM_FOREGROUND` ya pide un `Scan` ante una ventana desconocida; las recién adoptadas (salvo las del arranque) pasan a `greet`. Descartados `EVENT_OBJECT_SHOW/CREATE` (todos los objetos de todas las apps) y `NAMECHANGE`. Plan B si hace falta: `RegisterShellHookWindow`. En reposo, 0 despertares nuevos; sin reglas, nada extra.

## 4. Datos y ajustes

Cuarto campo de la línea de `apps.txt` (hereda la identidad de la letra; compatible en los dos sentidos):

```
b = brave | C:\...\brave.exe | Brave | al abrir: derecha
n = notepad | | | al abrir: centrada 60
```

Globales en `settings.txt`: `espacio_entre_ventanas` y `abrir_en_la_pantalla_del_puntero`. Ajustes: fila "Al abrir:" bajo "abre" (solo si la letra tiene app), marca gris en la lista ("izq.", "máx."), dos interruptores nuevos, y la ventana en dos pestañas ("Letras", "Opciones").

## 5. Pasos

0. Observación sin mover nada: log por ventana nueva (principal o no, única o no, cuándo quedó quieta) con Brave, AutoCAD, Revit, IntelliJ, Steam.
1. Formato (`rules.rs`, cuarto campo).
2. Aplicar (editando `apps.txt` a mano).
3. Ajustes.
4. `ci-probe` con un títere de otra app.
5. (b) Puntero.
6. (d) Espacios.
7. Documentación.

## 6. Decisiones

Ver §7 (se completa con las respuestas del usuario).

## 7. Decisiones del usuario (17/11/2026)

- A qué ventanas: **solo la primera** ventana principal de la app (la segunda de Brave, popups y pestañas arrancadas quedan donde las pone la app).
- Desplegable: **5 opciones** (no hacer nada, mitad izquierda, mitad derecha, maximizada, centrada 80 %); el resto solo a mano.
- Monitor: **donde la abre la app**; "abrir en la pantalla del puntero" como interruptor opcional, apagado.
- Espacios: **elegir el tamaño** (0–24 px), no un interruptor fijo.
- Con la recomendación de Fable: Z deshace la regla; sin animación al aplicar; si la app se reubica sola, gana la app; volver de la bandeja cuenta como abrir; reglas solo para apps con letra; ajustes en dos pestañas; (c), (e) y recordar posición, para después.

## 8. Avance

- **Pasos 1–3 hechos y probados por el usuario (17/11/2026):** Steam desde la bandeja (maximizada) e IntelliJ desde cero (mitad derecha) aplican la regla solos; Z la deshace.
- Lo que salió de las pruebas:
  - IntelliJ (Java) activa su ventana antes de terminar de mostrarla: el `Scan` del `FOREGROUND` no la ve y Windows no vuelve a avisar. Con reglas, wm la vuelve a buscar (cada 250–500 ms los primeros 3 s, hasta 10 s). En las pruebas apareció a los 1.5–2.5 s.
  - Una ventana nueva que aparece detrás (la pantalla de carga al frente) queda anotada 5 s: si pasa al frente, la regla corre; si no, no se toca.
  - El `Scan` del seguimiento tras abrir (Caps + letra) también adopta y aplica la regla.
  - Steam (CEF) vuelve de la bandeja con la marca de maximizada y la ventana a tamaño normal: solo cuenta como maximizada si cubre el área de trabajo (también en Caps + Alt + F).
  - Cada vez que la regla no corre, el log dice por qué.
- **Pasos 4–7 hechos (17/11/2026):** `ci-probe` con `wmregla.exe` (corre en la CI), puntero, espacios, ajustes en dos pestañas y documentación (README, `plan-core.md` §11, `keyboard-design.md` §6.4). El usuario probó puntero y espacios (8, 24 y 0 px; reacomodo al cambiarlo; A/D/esquinas/G/V/C/F y empujar al otro monitor) sin fallos.

## 9. Diseño de espacios y puntero (Fable 5.2, 17/11/2026)

**Espacios (d).** `g` = valor elegido (0–24) × DPI/96, limitado a 1/16 del lado menor. Afuera `g`, adentro `g` en total (`g/2` por lado; el píxel impar a la pieza derecha/de abajo). Tablero `B` = área de trabajo menos `g` por lado; `slot_rect(m, p, g)` y `slot_of(m, frame, g)` en place.rs. Maximizar, pantalla completa y C (centrar) siguen contra el área de trabajo; G/V contra `B`. `anchor`, `still_there` y la detección de bordes de `resize` reciben `B`. La detección (encaje, ajuste de 1 px, adopción, reencaje 4d) compara con `slot_rect` del `g` vigente. Si cambia `g`, las ventanas encajadas con el `g` viejo se reacomodan una vez (`Survey` + `go` exacto, sin vuelo ni Z); las demás, con la próxima tecla. Solo 14 llamadas de producción cambian; pruebas, sonda y demos siguen con `place`.

**Puntero (b).** `pointer: Option<(i32, i32)>` en `Inspection`/`Obs` (`GetCursorPos` en `inspect`). Con la opción, más de un monitor y puntero: la regla se aplica en el monitor del puntero. Solo ventanas con regla. Maximizada en otro monitor: sacar la marca, colocarla centrada en el destino y maximizar (como `undo`).

**Ajustes.** Pestaña Opciones, tarjeta "Ventanas": `Select` de espacio (sin espacio, 4, 8, 12, 16, 24; un valor a mano va como séptima opción) y el interruptor del puntero, visible solo con más de un monitor.
