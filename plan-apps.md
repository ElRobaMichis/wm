# `Caps` + letra con apps reales: bandeja, identidad, ajustes y ciclo

Diseño de Fable 5.2 (16/11/2026) a partir de la prueba del usuario; decisiones del usuario al final. Nada de esto está implementado todavía (ver avance en §7).

## 1. Problemas encontrados por el usuario

1. **Steam en la bandeja:** `Caps` + S decía "no hay ninguna app abierta con la S". Solo cuentan las ventanas de usuario; en la bandeja hay 0, y "si no está abierta, la abre" (keyboard-design §6.1) solo existe si la asignación trae una ruta, que los ajustes nunca escriben.
2. **`s = steam` nunca funciona:** la ventana de Steam es de `steamwebhelper.exe` (hijo de `steam.exe`); `resolve` compara el nombre exacto y, con la letra asignada, ya no prueba la inicial. Misma clase: lanzadores, Electron, apps de la Store.
3. **Ajustes:** lista fija de ~40 apps en orden arbitrario, sin búsqueda; faltan las que el usuario usa; escribir rutas no es para una persona no técnica.
4. **Misma letra:** sin asignar, solo la primera app en orden Z (Notepad era inalcanzable con N si Nanofy era más reciente); repetir iba a la ventana **menos** reciente (con 10 de Brave, volver a la anterior costaba 9 toques).

## 2. Identidad de una app (punto 2)

Una ventana es de la asignación si:
1. su exe está entre los nombres de la asignación (más alias: `s = steam, steamwebhelper`);
2. el AUMID de su proceso es el del destino `shell:AppsFolder\…` (apps MSIX/Store);
3. su **familia** está entre los nombres: el exe del ancestro más alto (≤ 3 niveles, validando la hora de creación) cuya carpeta contiene al exe de la ventana y que no está en `%SystemRoot%`. Steam cumple; Notepad abierto por Rayo no.

Asignada, sin ventanas y sin destino → se comporta como sin asignar (red de seguridad). Solo lecturas de proceso, con caché por pid; nada en el hook. Límite aceptado: los juegos dentro de `\Steam\steamapps` cuentan como Steam.

## 3. Abrir o mostrar (punto 1)

Letra asignada sin ventanas = "hacer clic en su ícono del menú Inicio": las de instancia única (Steam, Discord, Spotify) muestran su ventana; una cerrada se abre. Destino: el de la asignación; si falta, el exe del proceso vivo con ese nombre; si no, aviso "No sé cómo abrir X: elegila de nuevo en Ajustes". Protecciones (AutoCAD y Revit): durante 30 s tras lanzar (o hasta ver su ventana) otra pulsación solo avisa "Abriendo X…"; no relanzar si el proceso tiene una ventana visible que no es de usuario (pantalla de carga); tras lanzar, seguir con `Scan` a 0,3 / 0,8 / 1,5 / 3 s y enfocar la ventana nueva si el foco no cambió. Letras **sin asignar**: las apps solo en la bandeja no cuentan en v1.

## 4. Ajustes (punto 3)

Módulo `wm-platform::catalog` con dos fuentes: **abiertas ahora** (ventanas de usuario agrupadas por familia) y **todas** (`shell:AppsFolder`: accesos directos de los dos menús Inicio y apps de la Store; sin URLs, sin desinstaladores). Selector en la misma ventana con búsqueda (`SearchField`) y secciones "Ninguna", "Abiertas ahora", "Todas" (A→Z). Al elegir se guardan los nombres (exe y familia), el destino (ruta del exe, o `shell:AppsFolder\<AppID>`) y el nombre visible.

`apps.txt` (compatible con los archivos actuales):

```
# letra = app[, otro exe…] | cómo abrirla (opcional) | nombre visible (opcional)
s = steam, steamwebhelper | C:\Program Files (x86)\Steam\steam.exe | Steam
c = claude | shell:AppsFolder\Claude_pzs8sxrjxfjjc!Claude | Claude
```

## 5. Varias ventanas o apps en una letra (punto 4): "el Alt+Tab de la letra"

Cada letra tiene una lista, de la ventana más reciente a la más vieja (entre todos los escritorios). Asignada: solo las ventanas de esa app. Sin asignar: todas las ventanas de las apps con esa inicial que no están asignadas a otra letra, en una lista. Primer toque: la más reciente; si ya estás en una de la lista, la anterior (ir y volver = un toque). Sin soltar `Caps`, repetir la letra avanza por la lista (congelada en el primer toque) y da la vuelta; soltar `Caps` confirma. Única ventana y ya al frente: aviso "Es la única ventana de X". Después: selector con títulos en el overlay, Shift + letra hacia atrás, "Brave · 10" en el mapa.

## 6. Decisiones del usuario (16/11/2026)

- Ciclo: **como Alt+Tab** (no el ciclo continuo de antes).
- Letra asignada: **solo esa app**.
- Sin asignar: **todas sus ventanas juntas**, por uso reciente.
- App viva sin ventana: **relanzar con protecciones**.
- Con las recomendaciones de Fable (sin preguntar): apps solo en la bandeja no cuentan para letras sin asignar; los juegos de Steam cuentan como Steam; `wm-settings` usa `wm-platform`; los nombres largos se recortan en el mapa.

## 7. Pasos

1. Modelo: formato de `apps.txt` y red de seguridad.
2. Modelo: ciclo "Alt+Tab de la letra".
3. Plataforma: ruta, familia y AUMID por ventana, con caché.
4. Lanzar: destino desde el proceso vivo, guardia de 30 s, pantalla de carga, avisos.
5. Seguimiento tras lanzar (re-`Scan` y foco).
6. `catalog` y `wm apps --instaladas`.
7. Ajustes: selector con búsqueda que guarda nombres, destino y nombre.
8. Nombre visible en mapa y avisos; README y keyboard-design.

**Avance (16/11/2026, sin subir; falta la prueba del usuario):** pasos 1–7 hechos. Modelo: formato nuevo de `apps.txt` (alias, destino, nombre), identidad por exe/familia/AUMID, "Alt+Tab de la letra", `Revive` con guardia de 30 s y seguimiento a 0,3/0,8/1,5/3 s (sin el chequeo de pantalla de carga: queda para después). Plataforma: `process_info` (familia y AUMID con caché por pid), `launch::open` (exe, `.lnk` y `shell:AppsFolder` con `explorer.exe`), `Op::Revive`, `catalog` (107 apps en 120 ms en la PC del autor), `wm apps --instaladas`. Ajustes: selector que cubre la ventana, con búsqueda y secciones "Ninguna", "Abiertas ahora" y "Todas"; guarda nombres, destino y nombre visible.

## 8. Elegir entre muchas ventanas de una letra (diseño de Fable 5.2, 16/11/2026)

Con 3 o más ventanas en la lista de una letra, el **segundo toque** sin soltar `Caps` entra en **modo lista**: no enfoca, mueve la selección; **soltar `Caps` = ir** (Alt+Tab real). El primer toque sigue siendo inmediato; con 1 o 2 ventanas, todo como antes. La lista aparece 150 ms después del segundo toque (al instante si el mapa ya se ve), **reemplaza al mapa** en el mismo overlay, en el monitor del frente, también sobre una app a pantalla completa. Teclas: la letra = siguiente (da la vuelta); ↑ ↓; Esc = cerrar y quedarse en la ventana que se ve; otra letra = cerrar e ir a esa app; dígito o Alt = cerrar sin elegir. Orden: el del primer toque (más reciente primero). Filas: título sin la parte común (" - Brave"), cortado con "…"; a la derecha "escritorio N", "minimizada" y la app si la letra mezcla apps. Tope de 30 s.

**Decisiones del usuario:** ir al soltar `Caps`; desde 3 ventanas; Esc se queda donde está; **solo títulos, sin miniaturas**. Con las recomendaciones de Fable: los dígitos no eligen filas; clic fuera de la lista cancela; accesibilidad (Narrador) queda para después.

Límites conocidos: la fuente del overlay (Geist) no tiene emoji ni CJK: esos caracteres se omiten hasta sumar un respaldo.
