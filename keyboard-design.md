# Diseño de atajos de teclado

Objetivo: que una persona pueda usar el gestor **sin leer documentación ni ver tutoriales**. Los atajos deben ser **pocos, lógicos, hacerse con una mano** y **funcionar en cualquier idioma de teclado**.

- **Origen:** la experiencia de construir un tiling para una persona no técnica, que se abrumó por la cantidad de comandos y lo dejó. Coincide con lo que se ve en Reddit (ver `findings.md` §6.3 y §11).
- **Estado:** **borrador para probar con usuarios reales.** Nada de esto está validado todavía. **Mapa oficial: "Apps primero"** (opción B, §6), con "Ventanas primero" como perfil opcional (§8.1). La decisión y sus motivos están en §14.2.
- Las notas marcadas **(nota propia)** son conocimiento técnico nuestro y hay que verificarlas en la implementación.

---

## Índice

1. [El problema](#1-el-problema)
2. [Principios](#2-principios)
3. [Análisis del teclado y de las manos](#3-análisis-del-teclado-y-de-las-manos)
4. [Idiomas y distribuciones de teclado](#4-idiomas-y-distribuciones-de-teclado)
5. [La tecla Hyper](#5-la-tecla-hyper)
6. [El mapa completo](#6-el-mapa-completo)
7. [Equivalencias con el mouse](#7-equivalencias-con-el-mouse)
8. [Perfiles alternativos](#8-perfiles-alternativos)
9. [Conflictos con el sistema operativo](#9-conflictos-con-el-sistema-operativo)
10. [Cómo se aprende sin documentación](#10-cómo-se-aprende-sin-documentación)
11. [Notas de implementación](#11-notas-de-implementación)
12. [Plan de prueba con usuarios](#12-plan-de-prueba-con-usuarios)
13. [Preguntas abiertas](#13-preguntas-abiertas)
14. [Revisión tras la tercera ronda: apps por su inicial](#14-revisión-tras-la-tercera-ronda-apps-por-su-inicial)

---

## 1. El problema

Los gestores de tiling actuales (AeroSpace, yabai, komorebi, GlazeWM, i3, Hyprland):

- **Exponen 40–50 comandos** con el mismo peso: enfocar, mover, dividir horizontal o vertical, apilar, redimensionar ±px… Copian a i3 y están pensados para quien arma su propio sistema.
- **No siguen ninguna lógica:** `Alt+Shift+;`, `Ctrl+Alt+Espacio+1`. No hay nada que los haga memorables.
- **Obligan a memorizar antes de que la app sirva para algo.** Si no los sabés, la app no hace nada.
- **Muchos requieren las dos manos** o posiciones forzadas, cuando quien usa mouse tiene la mano derecha ocupada.
- **Se diseñan con teclados de EE. UU.** y rompen en otros idiomas (ver §4).

**La realidad del uso:** el 80–90 % del día son unas **6 a 10 acciones**: mitad izquierda o derecha, maximizar, centrar, mover a otro escritorio o monitor, cambiar de ventana y deshacer.

**Los modelos a imitar:**
- **Los juegos:** WASD es movimiento por **posición**; I = Inventory y R = Reload por **significado**; `Shift` siempre es "la variante" (correr). El jugador aprende el sistema una sola vez.
- **`Win+E`** abre el Explorer: la letra es la palabra.

### 1.1 Evidencia que respalda este diseño

(Detalle en `findings.md` §16).

- **Un usuario de Sway con 188 votos ya usa casi este mismo esquema:** *"I use Super, Shift, 0-9, and WASD (plus some Q and E – Super+Q swaps to the previous active workspace) to switch."* WASD, Q/E y números surgen solos en quien optimiza a mano.
- **Pocos atajos alcanzan:** *"there are like less than 10 keystrokes I actually needed to learn"* (usuario de dwm desde hace años).
- **Acostumbrarse rápido es posible** con buenos valores por defecto: *"I could get used to [Omarchy] already after the first 3 days."*
- **El caso GlazeWM (video):** la persona **no sabía cuál era la tecla de activación**, tuvo que **descargar la imagen de los atajos** desde la web y **editar un archivo con buscar y reemplazar** para cambiar el modificador. Justifica el mapa al mantener Hyper (§10.1) y el modificador configurable desde la interfaz (§5.2).

---

## 2. Principios

1. **Núcleo chico:** como máximo ~10 acciones para aprender, que salen de **3 reglas**.
2. **1 modificador + 1 tecla** para todo lo frecuente. Un segundo modificador solo para la **variante** de una acción que ya conocés, y siempre el mismo.
3. **Una sola mano (la izquierda)** para todo el núcleo. La derecha puede quedarse en el mouse.
4. **Cada atajo tiene un porqué:** o por **posición** (la tecla está donde apunta la acción) o por **significado** (la letra es la palabra, en español **y** en inglés).
5. **Repetir en lugar de agregar:** apretar el mismo atajo otra vez da la siguiente variante (½ → ⅔ → ⅓). Menos teclas, más resultados.
6. **Nunca chocar** con los atajos del sistema, de las apps ni con la escritura de caracteres (§4.1).
7. **Nunca hace falta memorizar:** mantener Hyper muestra el mapa (§10).
8. **Todo lo del núcleo se puede hacer con el mouse.** El teclado es un atajo, no un requisito.
9. **Lo poco frecuente no ocupa un atajo:** va al buscador de comandos.
10. **Configurable, pero con valores por defecto pensados.** Casi nadie cambia lo que viene por defecto.

---

## 3. Análisis del teclado y de las manos

### 3.1 Posición de mecanografía

```
 `  1  2  3  4  5  6  7  8  9  0
 Tab  Q  W  E  R  T  Y  U  I  O  P
 Caps  A  S  D [F] G  H [J] K  L  Ñ/;
 Shift  Z  X  C  V  B  N  M  ,  .  -//
 Ctrl  Win/Opt  Alt/Cmd  [  Espacio  ]  ...
```

| Dedo (mano izquierda) | Tecla en reposo | Alcanza fácil |
|---|---|---|
| Meñique | A | `Caps`, `Shift`, `Tab`, `Ctrl`, Q, Z, 1 |
| Anular | S | W, X, 2 |
| Medio | D | E, C, 3 |
| Índice | F | R, T, G, V, B, 4, 5 |
| Pulgar | Espacio | `Cmd` (Mac), `Alt` (Windows), Espacio |

**Observaciones:**
- **Los pulgares** son los dedos más fuertes y casi no se usan. En **Mac**, `Cmd` queda justo debajo del pulgar: es el mejor modificador del teclado. En **Windows** queda `Alt`; `Win` obliga a doblar el pulgar.
- **`Caps Lock`** está en la fila guía, al lado del meñique, y casi nadie lo usa. **Es el mejor lugar libre del teclado.**
- **Las flechas** obligan a soltar el mouse o a mover toda la mano derecha.
- **El meñique solo puede apretar una tecla a la vez.** Si mantiene `Caps`, no puede apretar también `Shift` ni `Tab`.

### 3.2 La "posición gamer"

Cuando el meñique mantiene `Caps`, la mano se corre **una tecla a la derecha**. Es exactamente la posición de los juegos:

```
        [W]
  Caps [A][S][D]      meñique: Caps  ·  anular: A  ·  medio: W/S  ·  índice: D
```

- Desde ahí se alcanzan sin esfuerzo: **Q, W, E, R, T / A, S, D, F, G / Z, X, C, V / 1–4** y el **pulgar** sobre `Cmd`/`Alt`/Espacio.
- El 5 llega con un estiramiento leve del índice.
- **Conclusión:** el núcleo vive en `Caps` + esa zona. Cualquiera que haya jugado a un juego de PC ya tiene la memoria muscular.

### 3.3 Costo de cada combinación

| Combinación | Teclas | Manos | Costo | Veredicto |
|---|---|---|---|---|
| `Caps + D` | 2 | 1 | Muy bajo | ✅ Núcleo |
| `Caps + pulgar + 2` | 3 | 1 | Bajo (el pulgar no compite con otros dedos) | ✅ Variante |
| `Caps + Shift + 2` | 3 | 1 | Alto (el meñique tendría que apretar dos teclas) | ⚠️ Solo como alias, con el `Shift` derecho |
| `Ctrl + Alt + flecha` | 3 | 1–2 | Medio, y choca con `AltGr` | ❌ |
| `Ctrl + Alt + Espacio + 1` | 4 | 1, forzada | Muy alto | ❌ |
| `Cmd + Shift + Option + ;` | 4 | 2 | Muy alto, y depende del idioma | ❌ |

---

## 4. Idiomas y distribuciones de teclado

### 4.1 ⚠️ Combinaciones que escriben caracteres

En teclados que no son de EE. UU., algunos modificadores **escriben símbolos**. Un atajo global ahí **le impide al usuario escribir ese símbolo**.

| Sistema | Distribución | Combinación | Escribe |
|---|---|---|---|
| Windows | Español (España) | `AltGr+2` = `Ctrl+Alt+2` | @ |
| Windows | Español (España) | `AltGr+3` / `AltGr+1` / `AltGr+E` | # / \| / € |
| Windows | Español (Latinoamérica) | `AltGr+Q` = `Ctrl+Alt+Q` | @ |
| Windows | Muchas europeas (alemán, francés…) | `AltGr+…` | {, [, ], }, \, ~, @, € |
| macOS | Español | `Option+2` / `Option+3` | @ / # |
| macOS | Casi todas | `Option+letra` | Caracteres especiales |

**Reglas:**
- ❌ **Nunca** atajos globales en `Ctrl+Alt+tecla` (Windows) ni en `Option+tecla` (macOS).
- ✅ Dentro de la capa Hyper (§5) no hay problema: la app captura `Caps` y consume toda la combinación, así que no llega a escribir nada.

### 4.2 Teclas que cambian de lugar

| Tecla | US (ANSI) | Español (ISO) | Otras |
|---|---|---|---|
| `` ` `` | Arriba de Tab | En otro lugar | Por eso `Cmd+`` no anda fuera de EE. UU. |
| `; ' [ ] \ /` | Directas | Se mueven o requieren `AltGr` | — |
| Ñ | — | Donde está `;` | — |
| Tecla junto a la Z | No existe | Existe (`<`) | — |
| Letras | QWERTY | QWERTY | **AZERTY**: A↔Q y Z↔W intercambiadas, M en otro lugar. **QWERTZ**: Y↔Z intercambiadas |
| Números | Directos | Directos | AZERTY: los números requieren `Shift` |

**Estable en todas las distribuciones:** la posición física de las teclas, las flechas, `Espacio`, `Tab`, `Enter`, `Esc`, los modificadores y casi todas las letras.

**Regla:** ❌ ningún atajo del núcleo usa símbolos (`` ` ; ' [ ] / \ , . - ``).

### 4.3 Dos tipos de atajo, dos formas de asignarlos

| Tipo | Ejemplos | Cómo se asigna | Por qué |
|---|---|---|---|
| **Por posición** | W/A/S/D, Q/E, números | **Por tecla física** (keycode o scancode) | Para que en AZERTY la tecla "de la izquierda" siga siendo la de la izquierda, aunque diga Q en vez de A |
| **Por significado** | F (Full), C (Centrar), Z (deshacer), R (Restaurar) | **Por carácter** según la distribución activa | Para que la C sea la C en cualquier teclado |

- El **mapa en pantalla** (§10.1) siempre muestra la **etiqueta real** de la distribución del usuario. En AZERTY, el bloque de dirección se ve como Z/Q/S/D.
- **Caso borde:** si una letra con significado cae en la misma tecla física que una de posición (pasa con AZERTY), gana la de posición y la de significado se reasigna. El mapa lo muestra.

### 4.4 Letras que funcionan en los dos idiomas

| Letra | Inglés | Español | ¿Sirve? |
|---|---|---|---|
| **F** | **F**ull / **F**ill | "**F**ull", pantalla completa | ✅ (en español se dice "full") |
| **C** | **C**enter | **C**entrar | ✅ |
| **R** | **R**estore | **R**estaurar | ✅ |
| **Z** | Undo (`Ctrl+Z`) | Deshacer (`Ctrl+Z`) | ✅ por convención universal |
| **X** | Cerrar (el ícono ✕) | Cerrar (el ícono ✕) | ✅ por símbolo |
| **T** | **T**ile | "**T**iling" | ✅ (el término se usa así en español) |
| M | **M**aximize | **M**aximizar | ✅, pero está en la mano derecha → solo alias |
| B | **B**rowser | Navegador | ❌ no sirve en español |
| N | New / Next | **N**uevo / **N**avegador | ❌ ambiguo |
| E | **E**xplorer | **E**xplorador | ✅, pero reservada para "siguiente" por posición (§6) |

---

## 5. La tecla Hyper

### 5.1 Qué es

Un modificador **exclusivo** del gestor, así no choca con nada. Por defecto es **`Caps Lock` mantenido**.

| Gesto | Resultado |
|---|---|
| Mantener `Caps` + otra tecla | Acción del gestor |
| Mantener `Caps` solo ~0,4 s | Muestra el **mapa de atajos** (§10.1) |
| Tocar `Caps` y soltar (sin otra tecla) | **Bloq Mayús normal** (por defecto), o `Esc`, o nada (configurable) |

### 5.2 Alternativas si el usuario necesita `Caps Lock`

| Sistema | Alternativa | Advertencias |
|---|---|---|
| macOS | **`Cmd` derecho** mantenido | Casi nadie lo usa; fswitcher lo usa para su cambio instantáneo |
| macOS | `Ctrl+Option` | Es el estilo de Rectangle. Choca con VoiceOver (usa `Ctrl+Option`) para quien lo tenga activado |
| Windows | **`Alt` derecho** **solo en distribuciones sin `AltGr`** (US) | ❌ En español y en la mayoría de las europeas el `Alt` derecho **es** `AltGr` |
| Windows | `Ctrl+Win` | Choca con `Ctrl+Win+←/→/D/F4` (escritorios virtuales). Hay que interceptarlos o evitar esas teclas **(a verificar)** |
| Windows | `Win+Alt` | Choca con varios atajos de Game Bar y del sistema (`Win+Alt+R/G/B/D/K`…) **(a verificar)** |
| Ambos | Cualquier tecla elegida por el usuario | Con prueba en vivo en la bienvenida ("apretá tu Hyper + A") |

**Recomendación:** `Caps` por defecto en las dos plataformas. En Windows la alternativa es más problemática que en Mac, así que hay que insistir más con `Caps`.

**Por qué no `Alt` solo, aunque GlazeWM lo recomiende:**
- En Windows, `Alt` solo activa los menús de las apps y `Alt+letra` abre menús (`Alt+F` = Archivo) **(nota propia)**.
- En el video de GlazeWM, la persona terminó cambiándolo por `Ctrl` editando el archivo a mano.

**El modificador se elige y se prueba desde la interfaz**, nunca editando un archivo.

### 5.3 Casos a detectar
- **macOS:** el ajuste "Usar Bloq Mayús para cambiar a y desde ABC" (Fuentes de entrada) hace que `Caps` cambie el idioma de escritura. Hay que detectarlo y avisar **(nota propia, verificar)**.
- **Usuarios de Karabiner, Hyperkey u otro remapeo de `Caps`:** detectarlo y ofrecer usar su Hyper existente (`Ctrl+Option+Cmd+Shift`) en vez de pelear por la tecla.

---

## 6. El mapa completo (oficial: "Apps primero")

> **Hyper** = `Caps Lock` mantenido. **Pulgar** = `Cmd` (Mac) / `Alt` (Windows), usado como segundo modificador **solo dentro de Hyper**.
>
> **Decisión:** este es el mapa oficial (la **opción B** de §14). La opción A no se descarta: queda como **perfil "Ventanas primero"** que el usuario puede elegir (§8.1). Ver los motivos en §14.2.

### 6.1 Las tres reglas

1. **Hyper + letra = la app.** La inicial de la app te lleva a ella; repetir pasa a su siguiente ventana; si no está abierta, la abre. (C = Chrome, S = Slack, T = Terminal…; las asignaciones las sugiere la app y las confirma el usuario, §6.4).
2. **Hyper + pulgar = la ventana.** Con el pulgar, las teclas mueven la ventana actual: **W A S D = dirección**, y **la letra es la palabra** (F = Full, C = Centrar, Z = deshacer).
3. **Números = escritorios.** Sin pulgar **vas** al escritorio; con pulgar **llevás** la ventana (coherente con la regla 2: el pulgar siempre actúa sobre la ventana).

**En una frase:** *"Caps + letra para ir; Caps + pulgar para acomodar."*

**Por qué las apps van sin pulgar:** cambiar de app es la acción más frecuente del día (decenas de veces); acomodar ventanas, unas pocas por sesión, y quien usa mouse lo hace arrastrando. Lo más frecuente tiene que ser lo más corto (§14.2).

### 6.2 Nivel 1: núcleo (se aprende en 5 minutos)

**Capa de apps (Hyper, sin pulgar)**

| Atajo | Acción | Al repetir | Por qué |
|---|---|---|---|
| **Hyper + letra** | **Ir a la app** asignada a esa letra (o abrirla) | Siguiente ventana de esa app | La inicial de la marca: igual en cualquier idioma |
| **Hyper + letra sin asignar** | Pregunta en el momento: *"¿Qué app querés en la K?"* y la asigna | — | Se configura usándolo, sin ir a ajustes |
| **Hyper + 1…4** | **Ir** al escritorio N | — | Números; el 5 es opcional |
| **Hyper + Espacio** | **Buscador de comandos** | — | Como Spotlight o Raycast |
| *Mantener Hyper* | **Mapa** con los íconos de las apps sobre cada tecla | — | §10.1 |

**Capa de ventana (Hyper + pulgar)**

| Atajo | Acción | Al repetir | Por qué |
|---|---|---|---|
| **Hyper + pulgar + A** | Ventana a la **izquierda** | **Empuja** al monitor de la izquierda (§14.4) | Posición (WASD) |
| **Hyper + pulgar + D** | Ventana a la **derecha** | Empuja al monitor de la derecha | Posición |
| **Hyper + pulgar + W** | Ventana **arriba** (mitad superior) | Empuja al monitor de arriba, si hay | Posición |
| **Hyper + pulgar + S** | Ventana **abajo** (mitad inferior) | Empuja al monitor de abajo, si hay | Posición |
| **Hyper + pulgar + W + A** (juntas o en secuencia) | **Esquina** superior izquierda | — | Diagonal, como en los juegos (§14.5) |
| **Hyper + pulgar + W + D / S + A / S + D** | Las otras esquinas | — | Idem |
| **Hyper + pulgar + F** | **Full**: maximizar (sin pantalla completa) | Vuelve al tamaño anterior | Significado; F está bajo el índice |
| **Hyper + pulgar + C** | **Centrar** | Alterna tamaños: 60 % → 80 % → original | Significado |
| **Hyper + pulgar + Z** | **Deshacer** el último movimiento | Sigue deshaciendo | Convención universal |
| **Hyper + pulgar + 1…4** | **Llevar** la ventana al escritorio N (sin seguirla) | — | Regla 3 |
| **Hyper + pulgar + Q / E** | Ventana **anterior / siguiente** de la zona o pila | Cicla | Q/E es "anterior/siguiente" en muchos juegos |

**Cuando el gestor está en modo zonas** (layouts definidos por el usuario), las mismas teclas hacen lo mismo conceptualmente:
- **A/D/W/S** = mover la ventana a la **zona vecina** en esa dirección. Si ya está en el borde de la pantalla, la empuja al monitor de al lado.
- **Q/E** = recorrer la **pila de ventanas** de esa zona.
- No hay que aprender un segundo esquema.

### 6.3 Nivel 2: útiles (aparecen en el mapa, no hace falta aprenderlos)

| Atajo | Acción | Por qué |
|---|---|---|
| **Hyper + pulgar + G / V** | **Agrandar / achicar** la ventana un paso (por ejemplo, 10 %) | G = **G**rande / **G**row; V queda debajo. **A validar** |
| **Hyper + pulgar + T** | **Tiling sí/no** para esta ventana (anclar o flotar) | Significado ("tile") |
| **Hyper + pulgar + R** | **Restaurar** el layout guardado del escritorio actual | Significado |
| **Hyper + pulgar + X** | **Cerrar** la ventana (no la app) | Símbolo ✕. **Desactivado por defecto** para evitar accidentes |
| **Hyper + pulgar + Shift + 1…4** | Llevar la ventana al escritorio N **y seguirla** | `Shift` = "y voy con ella" **(a definir)** |
| **Hyper + flechas** | Mover el **foco** a la ventana en esa dirección (para quien usa solo teclado) | Posición; usa la mano derecha |

- **Mover a otro monitor no necesita un atajo propio:** repetir la dirección empuja la ventana (§14.4).
- **Los grupos de apps** (abrir un "contexto" completo) van al buscador hasta que la prueba diga si merecen un atajo.

### 6.4 Asignación de letras a apps

- **Todas las letras sin pulgar son para apps.** Los números, `Espacio` y las flechas quedan reservados.
- **Sugerencia automática:**
  - en la bienvenida se proponen las apps más usadas (o las del Dock o la barra de tareas) con su inicial,
  - y el usuario **confirma con un clic**. Nunca se asigna sin preguntar.
- **Choques** (Slack, Safari y Spotify en la S):
  - se propone la siguiente letra significativa (S**p**otify → P, **N**avegador si el usuario lo prefiere),
  - o el usuario elige cuál se queda con la S.
- **Asignar en el momento:** apretar Hyper + una letra libre abre un selector de apps para esa letra.
- **Una letra puede apuntar a una categoría** en vez de a una app: "B = mi navegador" (el que esté como predeterminado). Así cambiar de Chrome a Arc no rompe la memoria muscular **(a validar)**.
- **El mapa siempre muestra el ícono** de la app sobre su tecla: no hay que recordar nada.

### 6.5 Nivel 3: solo en el buscador de comandos (sin atajo)

- Elegir o editar un layout de zonas.
- Guardar el layout actual.
- Abrir o cambiar de grupo de apps.
- Intercambiar el contenido de dos monitores.
- Proporciones exactas, espacios entre ventanas, márgenes.
- Reglas por app (flotar, ignorar, zona preferida, tamaño mínimo o máximo, a qué escritorio va al abrirse).
- Activar o desactivar el gestor por monitor.
- Pausar el gestor (también desde la barra de menú o la bandeja).
- Cualquier acción del nivel 1 o 2, buscándola por nombre.

### 6.6 Vista gráfica

```
CAPA DE APPS: Caps mantenido (ejemplo de asignación)

   [1 ir][2 ir][3 ir][4 ir][ 5 ]
 [Q  — ][W WhatsApp][E Excel][R  — ][T Terminal]   ...mano derecha: [N Notion][M Mail]...
[CAPS=HYPER][A Arc][S Slack][D Discord][F Finder][G — ]
       [Z Zoom][X Xcode][C Chrome][V VS Code][B — ]
                                     [ Espacio = buscador ]

CAPA DE VENTANA: Caps + pulgar mantenidos

   [1 llevar][2 llevar][3 llevar][4 llevar][ 5 ]
 [Q ◀ ant.][W ▲ arriba][E sig. ▶][R restaurar][T tiling]
[CAPS=HYPER][A ◀ izq.][S ▼ abajo][D der. ▶][F full][G agrandar]
       [Z deshacer][X cerrar*][C centrar][V achicar][ B ]
                     [ pulgar ]

* desactivado por defecto
```

### 6.7 Resumen de esfuerzo

| Nivel | Qué | Hay que memorizarlo | Teclas | Mano |
|---|---|---|---|---|
| 1 (apps) | Ir a 5–8 apps, ir a escritorios | **No**: la inicial ya la sabés | 2 | Izquierda (+ derecha según la letra) |
| 1 (ventana) | ~9 acciones | Sí, pero son 2 reglas (WASD + la letra es la palabra) | 3 | Izquierda |
| 2 | ~6 | No (aparece en el mapa) | 3–4 | Izquierda (+ flechas opcional) |
| 3 | Sin límite | No (buscador) | — | — |

---

## 7. Equivalencias con el mouse

Todo el nivel 1 se puede hacer sin teclado.

| Acción | Con el mouse |
|---|---|
| Ir a una app | Dock o barra de tareas, como siempre (el gestor la lleva a su escritorio o zona si tiene regla) |
| Asignar una letra a una app | Desde el mapa al mantener Hyper: arrastrar el ícono de la app a una tecla |
| Ventana a una mitad o esquina | Arrastrar al borde o a la esquina (sin retraso) |
| Ventana a una zona | Arrastrar **con clic derecho apretado** o **con Hyper**: aparecen las zonas |
| Maximizar | Doble clic en la barra de título / arrastrar arriba |
| Centrar | Arrastrar al centro superior (a definir) |
| Cambiar de ventana en la pila | Clic en la pestaña o en el indicador de la zona |
| Ir a otro escritorio | Clic en el indicador de la barra de menú (Mac) o de la bandeja del sistema (Windows), o gesto del trackpad |
| Llevar a otro escritorio o monitor | Arrastrar al indicador del escritorio o al otro monitor |
| Redimensionar dos ventanas juntas | Arrastrar el divisor compartido |
| Deshacer | Menú de la barra de menú o de la bandeja → "Deshacer" |
| Mover o redimensionar desde cualquier parte | **Hyper + arrastrar** (mover) / **Hyper + clic derecho + arrastrar** (redimensionar), como Swift Shift |

---

## 8. Perfiles alternativos

Los perfiles cambian **dos cosas independientes**: qué capa queda más a mano (§8.1) y qué teclas marcan la dirección (§8.2). Se eligen en la bienvenida y se cambian en ajustes.

### 8.1 ¿Qué querés más a mano?

| Perfil | Hyper + letra (2 teclas) | Hyper + pulgar + letra (3 teclas) | Para quién |
|---|---|---|---|
| **Apps primero** (por defecto, opción B) | Ir a la app | Acomodar la ventana | La mayoría: cambia de app todo el día y acomoda con el mouse |
| **Ventanas primero** (opción A) | Acomodar la ventana (WASD, F, C, Z…) | Ir a la app | Quien viene de tilers y acomoda ventanas con el teclado constantemente |

- **Es el mismo mapa con las capas intercambiadas.** Las reglas (WASD, la letra es la palabra, números = escritorios) no cambian; solo cambia cuál lleva pulgar.
- **En "Ventanas primero"**, los números siguen la misma lógica: sin pulgar vas, con pulgar llevás la ventana.
- **Cómo se ofrece sin abrumar:**
  - en la bienvenida es **una sola pregunta**, con "Apps primero" ya marcado: *"¿Qué hacés más seguido: cambiar de app o acomodar ventanas?"*;
  - si el usuario importa la configuración de AeroSpace, yabai, komorebi o GlazeWM (§14.6), se sugiere "Ventanas primero";
  - después se cambia en ajustes, con el mapa mostrando el resultado en vivo.
- **Riesgo:** cada opción extra es una decisión más para alguien que no quiere decidir. Por eso hay un valor por defecto claro y la pregunta se puede saltar.

### 8.2 Teclas de dirección (para la capa de ventana)

| Perfil | Dirección | Siguiente/anterior | Para quién |
|---|---|---|---|
| **WASD** (por defecto) | W A S D | Q / E | Usa mouse o trackpad, o es gamer |
| **Flechas** | ← ↑ ↓ → | `,` / `.` (posición física) | Quien viene de `Win+flecha`, Rectangle o Magnet |
| **Vim** | H J K L | U / I **(a definir)** | Quien viene de i3, Hyprland o AeroSpace |

- **Las letras de significado** (F, C, Z, R, T, X, G/V) y **la regla del pulgar** son iguales en los tres perfiles.
- **En "Apps primero" no hay choque** entre las teclas de dirección y las letras de las apps: viven en capas distintas (con y sin pulgar).
- El perfil **Vim** usa la mano derecha. Hyper conviene que quede en la izquierda (`Caps` sigue sirviendo).

---

## 9. Conflictos con el sistema operativo

La capa Hyper evita casi todos estos conflictos. La lista sirve si el usuario elige una alternativa a `Caps` (§5.2) y para no pisar los gestos nativos.

### 9.1 macOS (no tocar)

| Atajo | Qué hace |
|---|---|
| `Cmd+Tab`, `` Cmd+` `` | Cambiar de app / de ventana |
| `Cmd+Espacio` | Spotlight |
| `Cmd+H / M / Q / W` | Ocultar / minimizar / salir / cerrar |
| `Ctrl+←/→` | Cambiar de Space |
| `Ctrl+↑/↓` | Mission Control / App Exposé |
| `Ctrl+1…9` | Ir al escritorio N (si está activado) |
| `Fn+Ctrl+flechas`, `Fn+Ctrl+F` | Tiling nativo (Sequoia+) |
| `Ctrl+Cmd+F` | Pantalla completa en muchas apps |
| `Ctrl+Cmd+Q` | Bloquear pantalla |
| `Ctrl+Cmd+Espacio` | Emojis |
| `Ctrl+Espacio` | Cambiar fuente de entrada (idioma) |
| `Cmd+Shift+3/4/5` | Capturas |
| `Option+tecla` | **Caracteres** (§4.1) |
| `Ctrl+Option+…` | VoiceOver (si está activo) |

### 9.2 Windows (no tocar)

| Atajo | Qué hace |
|---|---|
| `Alt+Tab`, `Alt+F4`, `Ctrl+Alt+Supr` | Cambiar / cerrar / seguridad |
| `Win+L` | Bloquear (no se puede interceptar) |
| `Win+D / E / R / I / A / N / V / X / .` | Escritorio, Explorer, Ejecutar, Configuración, Configuración rápida, Notificaciones, Portapapeles, Menú avanzado, Emojis |
| `Win+Tab` | Vista de tareas |
| `Win+flechas`, `Win+Z` | Snap nativo |
| `Win+Shift+flechas` | Mover la ventana a otro monitor |
| `Win+Shift+S` | Recortes |
| `Win+1…9` | Abrir la app N de la barra de tareas |
| `Win+Ctrl+←/→/D/F4` | Escritorios virtuales |
| `Win+Alt+…` | Game Bar y otros (R, G, B, D, K…) **(a verificar)** |
| `Ctrl+Alt+tecla` | **`AltGr` → caracteres** (§4.1) |

### 9.3 Apps populares

Ningún atajo del núcleo debería tener **solo** `Cmd`/`Ctrl` como modificador: chocaría con las apps (`Ctrl+W`, `Ctrl+T`, `Ctrl+Tab`, `Cmd+1…9` en navegadores, etc.). La capa Hyper lo evita por diseño.

---

## 10. Cómo se aprende sin documentación

### 10.1 El mapa al mantener Hyper (la pieza clave)

- **Se activa** manteniendo Hyper solo ~0,4 s. **Desaparece** al soltar.
- **Muestra un teclado completo** dibujado con la **distribución real del usuario** (español, US, AZERTY…).
- **Sin pulgar muestra la capa de apps:** el **ícono de cada app** sobre su letra, los escritorios sobre los números y las letras libres con un "+" para asignar.
- **Al apretar el pulgar cambia a la capa de ventana** en el momento: flechas, "Full", "Centrar", "Deshacer"… Así se descubre la segunda capa sin explicarla.
- **Colores por grupo:** apps / dirección / acciones / escritorios.
- **Se puede usar sin soltar:** con el mapa abierto, apretar una tecla ejecuta la acción. Así el mapa enseña y funciona a la vez.
- **Se puede editar:** arrastrar el ícono de una app a otra tecla cambia su letra.
- **Abajo** muestra las reglas en una línea: *"letra = app · pulgar = ventana · números = escritorios"*.
- **Inspiración:** which-key (Vim/Emacs), la guía de atajos de Windows y los menús que aparecen al mantener `Cmd` en algunas apps.

### 10.2 Enseñar en el momento
- Cuando el usuario hace algo con el mouse que tiene atajo, aparece un aviso discreto: *"Esto también es Caps + pulgar + A"*.
- Cuando cambia a una app con `Cmd+Tab` o el Dock y esa app tiene letra: *"La próxima vez: Caps + C"*.
- **Como máximo** un aviso cada X minutos, solo para acciones que el usuario repite, y con opción de apagarlos para siempre.
- **Después de N usos** del atajo, ese aviso no vuelve a aparecer.

### 10.3 Bienvenida con casos reales
- En vez de una lista de comandos, la persona elige **qué hace**:
  - "Programar": editor a la izquierda (⅔), navegador a la derecha.
  - "Estudiar o leer": PDF y notas lado a lado.
  - "Comunicarme": mail y chat en un escritorio aparte.
  - "Monitor ultrawide": centro y dos laterales.
- **Se arma en vivo con sus ventanas reales**, mostrando qué tecla hizo cada cosa.
- **Confirmar las letras de sus apps** (sugeridas automáticamente, §6.4).
- **Mini ejercicio** de 3 pasos:
  1. "Apretá Caps + la inicial de tu navegador".
  2. "Ahora Caps + pulgar + A: tu navegador se va a la izquierda".
  3. "Ahora mantené Caps": aparece el mapa. Listo.

### 10.4 El buscador de comandos
- `Hyper + Espacio` → se escribe en lenguaje natural: "mover a monitor 2", "guardar layout", "flotar esta ventana".
- Cada resultado **muestra su atajo** si lo tiene, así el buscador también enseña.

---

## 11. Notas de implementación

Todas son **(nota propia)**: hay que verificarlas al implementar.

### 11.1 macOS
- **Capturar `Caps`:**
  - remapear `Caps Lock` a una tecla sin uso (por ejemplo F18) con `hidutil property --set` (UserKeyMapping), y tratarla como modificador desde un `CGEventTap`;
  - restaurar el mapeo al salir de la app.
  - Así lo hacen apps como Hyperkey. Verificar los permisos requeridos (Accesibilidad / Monitoreo de entrada).
- **Asignar por posición:** los códigos de tecla virtuales de macOS (`kVK_ANSI_A`, etc.) representan la **posición física**, que es justo lo que se necesita para WASD/Q/E/números.
- **Asignar por significado y dibujar el mapa:** traducir el código a carácter con la distribución activa (`TISCopyCurrentKeyboardLayoutInputSource` + `UCKeyTranslate`) y escuchar los cambios de fuente de entrada.
- **Secure Input:** con un campo de contraseña enfocado, el event tap no recibe teclas. Hay que mostrar un aviso discreto en lugar de "no hacer nada".

### 11.2 Windows
- **Capturar `Caps`:** un hook de teclado de bajo nivel (`WH_KEYBOARD_LL`) puede suprimir `Caps` y tratarlo como modificador **sin permisos de administrador** (el remapeo por registro "Scancode Map" requiere administrador y reiniciar).
- **UIPI:** con una ventana de un proceso elevado enfocada, un hook sin elevar **no recibe** las teclas. Hay que avisar al usuario u ofrecer ejecutar el gestor elevado.
- **Asignar por posición:** usar **scancodes**, no códigos virtuales (los VK dependen de la distribución).
- **Asignar por significado y dibujar el mapa:** `GetKeyboardLayout` + `ToUnicodeEx` / `MapVirtualKeyEx`, y escuchar los cambios de distribución.
- **`Win+L` y `Ctrl+Alt+Supr`** no se pueden interceptar. No intentarlo.

### 11.3 Ambos
- **Combinación de dos teclas (W+A):** detectar las dos dentro de una ventana corta (~50–80 ms) **o** mientras ambas estén apretadas. Ejecutar al **soltar** o con un pequeño retraso, para no disparar primero "arriba" y después "esquina". Hay que probar la sensación.
- **Repetir para alternar (½ → ⅔ → ⅓):** el ciclo se reinicia si pasa más de N segundos o si la ventana se movió por otro medio.
- **Deshacer:** guardar un historial de posiciones y tamaños por ventana (últimos ~20 movimientos).
- **Tocar `Caps` solo:** decidir qué es "toque" (soltar sin otra tecla en < ~300 ms) y respetar el comportamiento elegido (Bloq Mayús / Esc / nada).
- **Exportar e importar la configuración** (texto plano) para quien quiera versionarla, sin que sea necesario.

---

## 12. Plan de prueba con usuarios

**Usuaria ideal para la primera prueba:** la pareja que abandonó el prototipo anterior. Es exactamente el perfil al que apunta este diseño.

### 12.1 Sesión 1 (30 minutos)
1. **Sin explicar nada**, mostrar solo la bienvenida de §10.3 y ver qué entiende.
2. **Tareas** (pensando en voz alta):
   - Ir al navegador, después al chat y volver al navegador, solo con el teclado.
   - Poner el navegador a la izquierda y otra app a la derecha.
   - Hacer el navegador más ancho.
   - Maximizar una ventana y volver atrás.
   - Llevar una ventana al escritorio 2 y volver.
   - Deshacer un movimiento.
   - Encontrar cómo centrar una ventana **sin ayuda** (¿descubre el mapa?).
3. **Observar:** qué atajos usa, cuáles busca en el mapa, dónde duda, si usa el mouse en su lugar.

### 12.2 Sesión 2 (al día siguiente, 10 minutos)
- Repetir las tareas **sin el mapa** primero y después con el mapa.
- **Medir:** cuántas acciones recuerda sin ayuda, tiempo por tarea y errores.

### 12.3 Uso real (1–2 semanas)
- **Registro local** de qué acciones usa y con qué frecuencia (solo en su máquina, con su permiso).
- **Preguntas al final:** ¿qué atajos siguen sin salirle? ¿cuáles nunca usó? ¿qué hizo con el mouse que hubiera querido con el teclado?

### 12.4 Criterios de éxito
- Al día siguiente, va a sus apps con `Caps + inicial` **sin mirar el mapa** (es lo mínimo esperable de la capa de apps).
- Recuerda **al menos 5 de las ~9** acciones de la capa de ventana al día siguiente.
- **Si la capa de ventana falla** (le cuesta el pulgar), se corrige solo esa capa; las apps por inicial no se tocan.
- **Si en algún momento se prueba "Ventanas primero"** con otra persona, comparar con las mismas tareas.
- Encuentra una acción desconocida **en menos de 10 segundos** con el mapa.
- **No abandona** el gestor después de una semana.
- Se repite con **2–3 personas más** de perfiles distintos (de Windows, gamer, que solo usa trackpad).

---

## 13. Preguntas abiertas

1. **¿Pulgar + F para Full o pulgar + M?** Con el mapa B, M queda libre en la capa de ventana; pero F está bajo el índice y M en la mano derecha.
2. **¿Las esquinas con dos teclas a la vez (W+A) se sienten bien** o generan disparos accidentales de "arriba"? Hay que prototipar.
3. **¿El pulgar (`Cmd`/`Alt`) es cómodo como "capa de ventana"?** Es la duda principal del mapa B. Alternativa si falla: `Shift` derecho.
4. **¿Cuántos escritorios por defecto: 4 o 5?** El 5 obliga a estirar el índice.
5. **¿Los grupos de apps merecen atajo** o alcanza con el buscador?
6. **¿Atajo para "centrar" en el mouse?** Falta definir el gesto.
7. **¿Qué hace un toque de `Caps` por defecto?** ¿Bloq Mayús (menos sorpresa) o nada?
8. **Perfil Vim:** definir anterior y siguiente.
9. **¿Cómo se ve el mapa en distribuciones raras** (Dvorak, Colemak)? Por posición funciona igual, pero las etiquetas pueden confundir.
10. **Modo "una ventana maximizada por escritorio"** (monocle o con pestañas): muchos usuarios avanzados trabajan así (`findings.md` §16.3).
    - ¿Debería ser un modo de primera clase?
    - ¿`Hyper + F` alterna solo la ventana, y `Hyper + pulgar + F` alterna el modo de todo el escritorio?
    - En ese modo, `Q/E` recorrería las ventanas maximizadas, igual que la pila de una zona.
11. **¿"Ir al escritorio anterior"** (como `Super+Q` del usuario de Sway)? Hoy Q/E son anterior/siguiente ventana. ¿Hace falta un atajo para volver al último escritorio usado?

---

## 14. Revisión tras la tercera ronda: apps por su inicial

(Evidencia en `findings.md` §17.)

### 14.1 Qué cambió

- **El usuario experto que más enseña sobre el tema** (Josean Martinez, autor de la guía que enlaza AeroSpace) pasó por yabai, después AeroSpace, y terminó en **Raycast con unos 9 atajos**. Su conclusión:
  - lo que más usa es **ir a una app por su inicial** (`Hyper + B` = navegador, `T` = terminal, `N` = Notion, `F` = Finder),
  - y lo segundo, **mandar la ventana a una mitad o maximizar**.
- En AeroSpace ya usaba **workspaces con letras** (B, M, N, P, V, T) porque *"we can use mnemonics to remember where our windows are"*.
- **Un comentario lo resume:** *"Window management via launcher is the best implementation imo, because I can never remember some hotkeys."*

**El problema del mapa actual (§6):**
- Las letras más cómodas se gastan en **acciones** (A, D, W, S, F, C, Z, R, T, X, Q, E) y las apps quedan con las sobras (B, G, V y la mano derecha).
- Pero las iniciales de las apps más usadas son **justo esas letras**:

| Letra | Apps que la "reclaman" |
|---|---|
| C | Chrome, Code (VS Code), Calendar, Claude, ChatGPT |
| S | Slack, Safari, Spotify |
| F | Finder, Figma, Firefox |
| T | Terminal, Teams, Telegram |
| A | Arc |
| D | Discord |
| W | WhatsApp, Word |
| Z | Zoom |
| X | Xcode, X |
| E | Excel, Edge |
| N | Notion, Notas |

- **Además, cambiar de app es muchísimo más frecuente** que acomodar ventanas: se acomoda pocas veces por sesión, pero se cambia de app decenas de veces. Y quien usa mouse acomoda arrastrando.

**Ventaja extra:** los nombres de las apps son **marcas**. Chrome empieza con C en español, en inglés y en cualquier idioma. Resuelve el problema del idioma (§4.4) mejor que las letras de acción.

### 14.2 Tres opciones

**Opción A: el mapa actual.**
- Hyper + letra = acción. Las apps van en las letras libres.
- ✅ Ya está diseñado.
- ❌ Las apps más usadas quedan con letras sin relación ("Chrome en la B").

**Opción B: separar por concepto (recomendada).**
- **Regla 1, sin pulgar = vos vas:**
  - **Hyper + letra** = ir a la app con esa inicial.
  - **Repetir** = siguiente ventana de esa app (como el "Cycle" de Josean, pero sin una tecla aparte).
  - Si la app no está abierta, la abre.
- **Regla 2, con pulgar = la ventana se mueve:**
  - **Hyper + pulgar + W/A/S/D** = ubicar la ventana.
  - **Hyper + pulgar + F / C / Z** = Full, centrar, deshacer.
  - **Hyper + pulgar + Q/E** = recorrer la pila de la zona.
- **Regla 3, números = escritorios:**
  - **Hyper + número** = ir.
  - **Hyper + pulgar + número** = llevar la ventana (encaja con la regla 2: el pulgar siempre actúa sobre la ventana).
- ✅ Cada concepto tiene su capa, y lo más frecuente (cambiar de app) es lo más corto (2 teclas).
- ❌ Ubicar ventanas pasa a ser de 3 teclas (meñique, pulgar y otro dedo; costo "bajo" según la tabla de §3.3).

**Opción C: acciones en Hyper, apps desde un buscador.**
- `Hyper + Espacio` y escribir el nombre (como Raycast o Spotlight).
- ✅ No hay que aprender nada para las apps.
- ❌ Es más lento que una tecla, y es lo que el usuario ya tiene con Spotlight o Raycast.

**✅ Decisión (noviembre 2026):**
- **B es el mapa oficial** (§6) y el valor por defecto ("Apps primero").
- **A queda como perfil opcional** ("Ventanas primero", §8.1), que en la práctica es el mismo mapa con las capas intercambiadas.
- **Se valida B con la prueba** (§12). Si la capa con pulgar falla, se corrige solo esa capa.

**Motivos**, comparando criterio por criterio:

| Criterio | A | B | Gana |
|---|---|---|---|
| Lo más frecuente (cambiar de app) es lo más corto | No | Sí | B |
| Memorabilidad ("C es Chrome" ya lo sabés) | Hay que aprender acciones | Las iniciales ya se saben | B |
| Independiente del idioma | A medias | Sí (son marcas) | B |
| Evidencia de usuarios reales (§17 de findings) | Parcial | Fuerte | B |
| Primer contacto para alguien no técnico | Dirección y acciones | "Caps + inicial" | B |
| Choques entre letras | No hay | Sí (se resuelven con sugerencias y el mapa) | A |
| Costo de acomodar ventanas | 2 teclas | 3 teclas (y el mouse lo cubre) | A |

### 14.3 Asignar las apps sin trabajo para el usuario

- **Asignación automática sugerida:**
  - tomar las apps que el usuario más usa (o las que tiene en el Dock o en la barra de tareas),
  - proponer su inicial,
  - y que el usuario **confirme con un clic**. Nunca se asigna sin preguntar.
- **Choques** (Slack, Safari y Spotify en la S):
  - proponer la segunda letra significativa (S**p**otify → P),
  - o dejar elegir cuál se queda con la S.
  - El mapa al mantener Hyper (§10.1) **muestra el ícono de la app** sobre su tecla, así no hay que recordar la asignación.
- **Teclas que nunca se asignan a apps:** las de la capa de ventana no chocan en la opción B, porque esa capa usa el pulgar. En la opción A sí habría que reservarlas.

### 14.4 Qué hace repetir una dirección

Hay dos expectativas que chocan:

| Comportamiento | Quién lo usa | A favor |
|---|---|---|
| **Ciclar tamaños** (½ → ⅔ → ⅓) | Loop, Rectangle, Raycast | Lo conocen los usuarios de Mac |
| **Empujar al monitor de al lado** | Windows (`Win+←` dos veces) | *"One hotkey can intuitively 'push' windows around, I don't need to memorize too many hotkeys."* Es coherente con el modo zonas (A/D ya mueven a la zona vecina y después al otro monitor) |

**Propuesta:**
- **Empujar** por defecto, porque es la misma lógica en modo libre y en modo zonas: "A siempre empuja a la izquierda".
- **Ciclar tamaños** como opción que se activa sola si el usuario importa la configuración de Rectangle, Loop o Raycast (§14.6).
- Para cambiar el ancho sin cambiar de lado: **agrandar o achicar** con la capa de ventana (por ejemplo, Hyper + pulgar + R / V, a definir), que también pide la gente (*"½ is not enough, ⅔ is too much"*).
- **A decidir con pruebas.**

### 14.5 Combinar direcciones en secuencia

- Además de apretar dos teclas a la vez (W+A, §6.2), aceptar la **secuencia**: D y enseguida W = cuarto superior derecho.
- Es lo que pidió un usuario de Raycast y lo que hace Swish con gestos (izquierda y después arriba = esquina superior izquierda).
- **Ventana de tiempo:** ~1 s desde el movimiento anterior **(a probar)**.

### 14.6 Importar los atajos que el usuario ya tiene

- En la bienvenida: **"¿De qué app venís?"** Rectangle, Magnet, Raycast, Loop, AeroSpace, el snap de Windows.
- Se cargan **sus mismos atajos** como perfil. Es lo que hizo Raycast y la gente lo celebró: *"you didn't have to learn anything at all"*.
- El mapa al mantener Hyper funciona igual con cualquier perfil.
- Para AeroSpace hay que leer su archivo TOML; para Rectangle, sus preferencias **(nota propia: verificar el formato)**.

### 14.7 Otras decisiones que salen de esta ronda

- **Hyper incluido en la app**, sin Karabiner (Raycast ya lo hace).
- **Borde o indicador de foco** prendido por defecto: los tutoriales de AeroSpace instalan JankyBorders porque sin él no se sabe qué ventana recibe los atajos.
- **El puntero sigue al foco:** opción para que el puntero salte al centro de la ventana enfocada con el teclado.
- **Llevar sin seguir:** `Hyper + pulgar + número` manda la ventana pero **te deja donde estás** (es lo que prefieren los dos autores de tutoriales). Para ir con ella, se suma `Shift` **(a definir)**.

### 14.8 Preguntas nuevas
1. ~~**¿Opción A o B?**~~ **Decidido:** B oficial, A como perfil opcional (ver §14.2).
2. **¿Repetir una dirección empuja o cicla tamaños** por defecto?
3. **¿Qué teclas para agrandar o achicar** la ventana?
4. **¿Repetir la letra de una app** recorre sus ventanas o la oculta (como `Cmd+H`)? rcmd y otras apps tienen las dos variantes.
