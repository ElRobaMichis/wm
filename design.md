# Lenguaje de diseño

Cómo se ve y cómo se siente `wm`: la ventana de ajustes, el mapa de teclado, el borde de foco, los avisos. La meta es la del usuario: **minimalista, con identidad propia y con atención al detalle, al nivel de Arc o Apple**, y nada de "slop" (varias fuentes, tamaños al azar, degradados, sombras pesadas).

Este documento sale de nueve referencias que eligió el usuario (15/11/2026). Primero, lo que enseña cada una; después, las reglas y los tokens.

---

## 1. Qué enseña cada referencia

| # | Referencia | Lo que hay que copiar |
|---|---|---|
| 1 | **Registro de horas** (oscuro, calendario + lista) | Superficies **anidadas**: la ventana, las tarjetas y, dentro, el encabezado un poco más oscuro, separadas por **bordes de 1 px apenas más claros**, no por sombras. **Una sola tipografía** y tres tamaños como mucho. La jerarquía la dan el **color** (texto principal o gris) y el peso, no el tamaño. Los números van a la derecha, con la unidad separada ("12 h"). La fila elegida va con un relleno apenas más claro. El foco es un **anillo azul de 2 px redondeado**, el único color fuerte. Lo destructivo (papelera) va en rojo apagado. |
| 2–3 | **Medidas en la grilla** (11,2 y 15,2) | **El espaciado es un sistema, no una sensación**: la misma distancia del ícono al borde de arriba, al de la izquierda y al texto. **Radios concéntricos**: el radio de adentro es el de afuera menos el relleno, así las curvas son paralelas. El ícono va centrado ópticamente en su caja. |
| 4 | **App de cafeína** (negro, monoespaciada) | **Números grandes con la unidad chiquita y en gris** ("158 MG"). Un **único acento** (azul) solo para "lo actual" o lo elegido. Separadores finísimos. Control de pestañas en **píldora** (la elegida, rellena). El botón principal es una **píldora clara** sobre fondo oscuro. Densidad alta sin ruido. |
| 5 | **Outly** (landing clara) | Mucho **aire**. Título en **dos tonos** (negro y su continuación en gris). Un solo botón negro con una sombra mínima. Tarjeta flotante con **una** sombra suave. |
| 6 | **Lista de proyectos** (clara) | Panel gris muy claro con radio grande. Al pasar el mouse, la fila es una **píldora gris** con el mismo radio interior. Los títulos de sección van **en gris, más chicos y semibold**. Los conteos, a la derecha y en gris. El color va solo en íconos chicos y pastel. |
| 7 | **Ruta al trabajo** (tarjeta blanca) | **Control segmentado**: un riel gris con el elegido en **píldora blanca con sombra apenas perceptible**. Número protagonista ("49") con la unidad chica. Metadatos alineados a la derecha. Una línea punteada de acento. |
| 8 | **Gestor de imágenes** (barra lateral) | Barra lateral **densa pero calma**: íconos de 16 px monocromos, títulos de sección en gris, conteos alineados a la derecha, la selección como píldora gris. Nada compite con el contenido. |
| 9 | **X rediseñado** (negro) | Navegación en minúsculas y **atajos de teclado alineados a la derecha en gris** ("write n", "search /", "tune ⌘k"). Encaja exacto con un producto que se maneja con el teclado como el nuestro. |

**Lo común:** monocromo más **un acento**, una tipografía, pocos tamaños, jerarquía por color y peso, superficies anidadas con radios concéntricos, bordes finos en lugar de sombras, metadatos alineados a la derecha, selección como píldora sutil y espacio generoso y medido.

---

## 2. Reglas

1. **Una tipografía.** Una sola familia sans. Los números con cifras **tabulares** para que no bailen. Nada de mezclar fuentes "porque queda lindo".
2. **Tres tamaños de texto, no más**: título, cuerpo y pie. Lo demás se resuelve con **color** (principal, secundario, terciario) y **peso** (normal o semibold).
3. **Un acento.** Azul, y solo para foco, "lo actual" y la acción principal en curso. Nunca decorativo. El rojo, solo para destruir.
4. **Sin degradados, sin vidrio, sin brillos.** Colores planos. Sombra únicamente en lo que **flota** (menús desplegables, avisos), y una sola, suave.
5. **Bordes antes que sombras.** Las superficies se separan por un borde de 1 px con opacidad baja o por un cambio de tono mínimo.
6. **Espaciado en una escala de 4 px** (4, 8, 12, 16, 24, 32). Cada distancia que se repite es **la misma** (como las medidas de las referencias 2 y 3).
7. **Radios concéntricos.** `radio interior = radio exterior − relleno`. Tarjeta de 14 con relleno de 6, filas de 8.
8. **Números a la derecha** con la unidad separada y en gris ("12 h", "4 apps"). Los **atajos de teclado** también a la derecha, como teclas chicas (referencia 9).
9. **La selección y el hover son píldoras** de tono apenas distinto, con el radio interior. Nada de bordes gruesos ni colores saturados para "seleccionado".
10. **El foco se ve siempre**: un anillo de acento de 2 px que sigue el radio del control. Es nuestra forma de respetar el teclado.
11. **Movimiento corto y con propósito**: 120–180 ms, curva de salida, solo en lo que cambia de estado (interruptores, píldora del control segmentado, hover). Respeta "reducir animaciones" del sistema.
12. **Texto en sentence case, sin signos de exclamación**, con voz de producto: corto y concreto ("12 de 26 letras asignadas").
13. **Claro y oscuro de verdad**: los mismos tokens con dos paletas, siguiendo el tema del sistema. No es un oscuro "invertido".
14. **Sin íconos decorativos.** Un ícono solo si ahorra palabras. Monocromo, trazo de 1,5 px y caja de 16 px.

**Anti-"slop", la lista negra:** degradados de colores; más de una familia tipográfica; tamaños que no estén en la escala; emojis en la interfaz; sombras grandes o de color; esquinas de radios distintos sin motivo; textos centrados en listas; mayúsculas sostenidas en todo; animaciones de rebote; "glassmorphism"; bordes de colores en todo; íconos de varios estilos mezclados.

---

## 3. Tokens

### 3.1 Tipografía

- **Familia: [Geist](https://github.com/vercel/geist-font)** 1.9.2, Regular y SemiBold, licencia OFL (`settings/ui/fonts/geist`, con su `OFL.txt`). Va dentro del binario, así que se ve igual en cualquier PC. La misma familia para el título y el cuerpo.
  - **Por qué Geist** (decisión del usuario, 15/11/2026): tiene carácter propio sin llamar la atención, es angosta y técnica, combina con las teclas y los atajos, y pesa un tercio que Inter. Segoe UI Variable es correcta pero es la de cualquier app de Windows.
  - **Pendiente:** el mapa de teclado y los avisos de `wm.exe` dibujan con DirectWrite y todavía usan Segoe UI. Para usar Geist hay que cargarla desde memoria (`IDWriteFactory5::CreateInMemoryFontFileLoader`), a costa de +254 KB en `wm.exe`.
  - La prueba (15/11/2026), con la ventana de ajustes y una opción `--font` que ya se quitó:

    | | Segoe UI Variable | Inter 4.1 | Geist 1.9.2 |
    |---|---|---|---|
    | Origen | del sistema | [rsms/inter](https://github.com/rsms/inter), OFL | [vercel/geist-font](https://github.com/vercel/geist-font), OFL |
    | Peso en el `.exe` (Regular + SemiBold) | 0 | +830 KB | +254 KB |
    | Arranque hasta el primer cuadro (p50, 20 vueltas) | 115 ms | 115 ms | 114 ms |
    | Memoria (WS privado) | 5,4 MB | 5,5 MB | 5,5 MB |
    | Carácter | la de Windows: no da identidad propia | neutra y muy legible; letra más ancha y alta, la más "producto" | geométrica, más angosta y técnica; combina con las teclas |

    Slint la incrusta con `import "fonts/…ttf"`: copia los bytes en el binario (se verificó buscándolos en el `.exe`).
- **Tamaños:** título 20 px semibold; cuerpo 13 px; pie 12 px.
- **Cifras:** ninguna de las tres trae cifras tabulares por defecto (medido en la tabla `hmtx`), y Slint 1.18 no permite activar rasgos OpenType como `tnum`. Por eso los números van **alineados a la derecha**: si cambian, no mueven lo que tienen al lado ("12 / 26").

### 3.2 Color

| Token | Oscuro | Claro | Uso |
|---|---|---|---|
| `bg` | `#0E0E0F` | `#F3F3F1` | Fondo de la ventana |
| `surface` | `#161618` | `#FFFFFF` | Tarjetas |
| `surface-header` | `#121214` | `#F8F8F7` | Encabezado de tarjeta (referencia 1) |
| `fill` | `#FFFFFF0F` (6 %) | `#0000000A` (4 %) | Hover, píldora elegida, campos |
| `fill-strong` | `#FFFFFF1A` (10 %) | `#00000012` (7 %) | Elegido + hover, riel de control |
| `border` | `#FFFFFF12` (7 %) | `#0000000F` (6 %) | Bordes de 1 px |
| `text` | `#EDEDEE` | `#141414` | Texto principal |
| `text-2` | `#8E8E93` | `#6E6E73` | Secundario (metadatos, unidades) |
| `text-3` | `#5A5A5F` | `#A1A1A6` | Terciario (deshabilitado, "libre") |
| `track-off` | `#FFFFFF26` (15 %) | `#00000026` (15 %) | Riel del interruptor apagado |
| `accent` | `#5B8CFF` | `#2F6BFF` | Foco, actual, interruptor encendido |
| `on-accent` | `#FFFFFF` | `#FFFFFF` | Texto sobre acento |
| `primary` | `#EDEDEE` | `#141414` | Botón principal (píldora clara en oscuro, oscura en claro) |
| `on-primary` | `#0E0E0F` | `#FFFFFF` | Texto del botón principal |
| `danger` | `#E5484D` | `#D93036` | Solo para destruir |

### 3.3 Espacio, radios y alturas

- **Espacio:** 4 · 8 · 12 · 16 · 24 · 32.
- **Radios:** ventana 14 (la de Windows 11 es 8; nuestras tarjetas, 14), tarjeta 14, filas y campos 8 (concéntricos con relleno de 6), teclas 6, píldora = altura / 2.
- **Alturas:** fila 36; campo 32; botón 32; interruptor 20 × 34.

### 3.4 Movimiento

- 140 ms, `ease-out`, para hover, la píldora del control segmentado y los interruptores.
- 0 ms si el sistema pide reducir animaciones (`SPI_GETCLIENTAREAANIMATION`).

---

## 4. Componentes

- **Tecla:** caja de 22 × 22 con radio 6, borde 1 px y la letra en cuerpo semibold. Es el sello visual de `wm`: las letras de `Caps` + letra se ven como teclas, en la lista y en el mapa.
- **Fila:** 36 px; tecla a la izquierda, nombre en el medio, metadato o atajo a la derecha en `text-2`. Hover y elegida como píldora (`fill` / `fill-strong`).
- **Campo de búsqueda:** `fill`, radio 8, lupa de 16 px en `text-2`, texto de ayuda en `text-3` y la tecla de atajo a la derecha.
- **Control segmentado:** riel `fill`, píldora elegida en `surface` con borde, y la píldora se desliza en 140 ms.
- **Interruptor:** riel 34 × 20; encendido = `accent`, apagado = `track-off`; perilla blanca de 16 con una sombra de 1 px.
- **Botón principal:** píldora `primary` / `on-primary`. Secundario: texto `text` sobre `fill`. Deshabilitado: `text-3`.
- **Menú desplegable:** tarjeta flotante (la única sombra), filas iguales a las de la lista.
- **Anillo de foco:** 2 px `accent` por fuera del control, con su radio + 2.

---

## 5. Aplicado a `wm`

- **Ajustes:** la pantalla del spike 8 rehecha con estas reglas (`settings/ui`).
- **Mapa de teclado (overlay):** las mismas teclas, la misma tipografía y los mismos tokens, en oscuro sobre cualquier fondo, con la tecla elegida en `accent`.
- **Borde de foco de ventanas:** 2 px `accent`, nunca otro color.
- **Avisos y bandeja:** las mismas filas y teclas; sin íconos de colores.
