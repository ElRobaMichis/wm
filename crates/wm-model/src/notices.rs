//! Avisos en pantalla (plan-core.md §10, `funciones-ocultas.md`): cuando una
//! tecla no hace nada, una línea corta en la pantalla además del log. Para
//! una persona no técnica, que no pase nada parece una falla.
//!
//! Se derivan de las líneas del log de las acciones del usuario: así cada
//! caso nuevo del modelo sigue logueando como siempre y el aviso sale de una
//! sola tabla, probada acá.

use crate::mapview::display_name;

/// El aviso para una línea del log, si corresponde mostrar uno.
pub fn notice_for(line: &str) -> Option<String> {
    let l = line;
    // "Caps + Q: no hay ninguna app abierta con la Q."
    if let Some(rest) = l.split("no hay ninguna app abierta con la ").nth(1) {
        let key = rest.trim_end_matches('.').trim();
        return Some(format!("Ninguna app abierta empieza con {key}"));
    }
    if l.contains("no hay otro monitor") {
        for side in ["a la izquierda", "a la derecha", "arriba", "abajo"] {
            if l.contains(&format!("no hay otro monitor {side}")) {
                return Some(format!("No hay otro monitor {side}"));
            }
        }
        return Some("No hay otro monitor de ese lado".into());
    }
    if let Some(before) = l
        .split(" está en pantalla completa")
        .next()
        .filter(|_| l.contains(" está en pantalla completa"))
    {
        let app = before.rsplit([' ', '(']).next().unwrap_or("").trim();
        return Some(if app.is_empty() {
            "La ventana está en pantalla completa".into()
        } else {
            format!("{} está en pantalla completa", display_name(app))
        });
    }
    // Hito 4d: "… Ventanas traídas a la pantalla: 2."
    if let Some(rest) = l.split("Ventanas traídas a la pantalla: ").nth(1) {
        let n: u32 = rest.trim_end_matches('.').trim().parse().unwrap_or(0);
        return Some(match n {
            1 => "Traje una ventana que había quedado fuera de la pantalla".into(),
            n => format!("Traje {n} ventanas que habían quedado fuera de la pantalla"),
        });
    }
    // plan-apps.md: el Alt+Tab de la letra y abrir apps.
    if let Some(app) = l.split(": ").nth(1).and_then(|r| r.strip_suffix(" es la única ventana.")) {
        return Some(format!("Es la única ventana de {}", display_name(app)));
    }
    if let Some(app) = l.split(": abriendo ").nth(1).and_then(|r| r.strip_suffix("… (espera)")) {
        return Some(format!("Abriendo {}…", display_name(app)));
    }
    if let Some(rest) = l.split(": no sé cómo abrir ").nth(1) {
        let app = rest.split(" (").next().unwrap_or(rest);
        return Some(format!(
            "No sé cómo abrir {}: elegila de nuevo en Ajustes",
            display_name(app)
        ));
    }
    if l.contains("la app no responde") {
        return Some("La app no responde: se va a mover cuando despierte".into());
    }
    for (pat, text) in [
        ("ya está en el tamaño mínimo", "Ya está en el tamaño mínimo"),
        ("ya está en el tamaño máximo", "Ya está en el tamaño máximo"),
        ("la app no se achica más", "Esta app no se achica más"),
        ("la app no se agranda más", "Esta app no se agranda más"),
        ("la ventana está minimizada", "La ventana está minimizada"),
        ("no hay nada para deshacer", "No hay nada para deshacer"),
        ("no hay una ventana al frente para llevar", "No hay una ventana para llevar"),
        ("esa ventana no se puede esconder", "Esta ventana no se puede llevar a otro escritorio"),
        ("no se puede cambiar (está maximizada)", "Primero sacala de maximizada"),
        ("no se puede cambiar.", "El tamaño de esta ventana no se puede cambiar"),
    ] {
        if l.contains(pat) {
            return Some(text.into());
        }
    }
    // "Caps + Alt + 2: ya está en el escritorio 2."
    if let Some(rest) = l.split("ya está en el escritorio ").nth(1) {
        return Some(format!("Ya está en el escritorio {}", rest.trim_end_matches('.').trim()));
    }
    // "Caps + B → no se pudo abrir brave (C:\…): …"
    if let Some(rest) = l.split("no se pudo abrir ").nth(1) {
        let app = rest.split([' ', '(']).next().unwrap_or("");
        return Some(format!("No se pudo abrir {}", display_name(app)));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_facing_failures_become_notices() {
        let cases = [
            ("Caps + Q: no hay ninguna app abierta con la Q.", "Ninguna app abierta empieza con Q"),
            (
                "Caps + Alt + A → mitad izquierda (brave): no hay otro monitor a la izquierda.",
                "No hay otro monitor a la izquierda",
            ),
            ("Caps + Alt + C: brave está en pantalla completa.", "Brave está en pantalla completa"),
            (
                "Caps + Alt + D → mitad derecha (code): la app no responde; va a llegar sola cuando despierte (930.0 ms)",
                "La app no responde: se va a mover cuando despierte",
            ),
            (
                "Caps + Alt + V: ya está en el tamaño mínimo (brave, 384×1032).",
                "Ya está en el tamaño mínimo",
            ),
            ("Caps + Alt + Z: no hay nada para deshacer.", "No hay nada para deshacer"),
            ("Caps + Alt + 2: ya está en el escritorio 2.", "Ya está en el escritorio 2"),
            (
                "Caps + clic derecho: el tamaño de brave no se puede cambiar (está maximizada).",
                "Primero sacala de maximizada",
            ),
            (
                "Caps + N → no se pudo abrir notion (C:\\x\\Notion.exe): no existe",
                "No se pudo abrir Notion",
            ),
        ];
        for (line, want) in cases {
            assert_eq!(notice_for(line).as_deref(), Some(want), "{line}");
        }
    }

    #[test]
    fn normal_lines_have_no_notice() {
        for line in [
            "Caps + Alt + A → mitad izquierda (brave): 0 px en 1 pasada, 4.4 ms desde la tecla",
            "Caps + B → brave «x»: ok en 1.2 ms (desde la tecla 2.0 ms)",
            "Mapa de teclado visible (monitor 1), 5.0 ms.",
            "Caps + 2 → escritorio 2: 1 mostradas, 2 escondidas, 9.0 ms desde la tecla",
        ] {
            assert_eq!(notice_for(line), None, "{line}");
        }
    }
}
