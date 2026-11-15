//! Traducciones desde archivos gettext `.po` (spike 8, architecture.md
//! §13.10). Los mismos `.po` que usa Slint (`settings/lang`) son la única
//! fuente: la interfaz nativa, la bandeja y la CLI los leen con esto.
//!
//! Alcanza para lo que usamos: `msgid`, `msgid_plural`, `msgstr` y
//! `msgstr[n]`, con plural de dos formas (`n != 1`), como el español y el
//! inglés.

use std::collections::HashMap;

#[derive(Debug, Default, Clone)]
pub struct Catalog {
    singular: HashMap<String, String>,
    plural: HashMap<String, [String; 2]>,
}

/// Contenido de una cadena entre comillas, con los escapes de gettext.
fn unquote(s: &str) -> String {
    let s = s.trim();
    let inner = s.strip_prefix('"').and_then(|x| x.strip_suffix('"')).unwrap_or(s);
    let mut out = String::new();
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some(other) => out.push(other),
                None => {}
            }
        } else {
            out.push(c);
        }
    }
    out
}

#[derive(Default)]
struct Entry {
    id: String,
    id_plural: Option<String>,
    str0: String,
    str1: String,
}

impl Catalog {
    pub fn parse(text: &str) -> Self {
        let mut cat = Catalog::default();
        let mut e = Entry::default();
        // Qué campo continúa una línea que empieza con comillas.
        let mut field = "";
        let flush = |e: &mut Entry, cat: &mut Catalog| {
            if !e.id.is_empty() {
                match e.id_plural.take() {
                    Some(_) if !e.str0.is_empty() => {
                        cat.plural.insert(
                            std::mem::take(&mut e.id),
                            [std::mem::take(&mut e.str0), std::mem::take(&mut e.str1)],
                        );
                    }
                    None if !e.str0.is_empty() => {
                        cat.singular.insert(std::mem::take(&mut e.id), std::mem::take(&mut e.str0));
                    }
                    _ => {}
                }
            }
            *e = Entry::default();
        };
        for line in text.lines().map(str::trim) {
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if let Some(rest) = line.strip_prefix("msgid_plural ") {
                e.id_plural = Some(unquote(rest));
                field = "plural";
            } else if let Some(rest) = line.strip_prefix("msgid ") {
                flush(&mut e, &mut cat);
                e.id = unquote(rest);
                field = "id";
            } else if let Some(rest) = line.strip_prefix("msgstr[0] ") {
                e.str0 = unquote(rest);
                field = "str0";
            } else if let Some(rest) = line.strip_prefix("msgstr[1] ") {
                e.str1 = unquote(rest);
                field = "str1";
            } else if let Some(rest) = line.strip_prefix("msgstr ") {
                e.str0 = unquote(rest);
                field = "str0";
            } else if line.starts_with('"') {
                let more = unquote(line);
                match field {
                    "id" => e.id.push_str(&more),
                    "str0" => e.str0.push_str(&more),
                    "str1" => e.str1.push_str(&more),
                    _ => {}
                }
            }
        }
        flush(&mut e, &mut cat);
        cat
    }

    /// Traducción de `id`, o `id` si no hay.
    pub fn tr<'a>(&'a self, id: &'a str) -> &'a str {
        self.singular.get(id).map_or(id, String::as_str)
    }

    /// Singular o plural según `n`, con `{n}` reemplazado.
    pub fn trn(&self, id: &str, plural: &str, n: i64) -> String {
        let form = usize::from(n != 1);
        let text = match self.plural.get(id) {
            Some(forms) => forms[form].as_str(),
            None if form == 0 => id,
            None => plural,
        };
        text.replace("{n}", &n.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PO: &str = r#"
msgid ""
msgstr ""
"Language: es\n"

# comentario
msgid "Save"
msgstr "Guardar"

msgid "Search an app or a letter…"
msgstr "Buscar app "
"o letra…"

msgid "{n} letter assigned"
msgid_plural "{n} letters assigned"
msgstr[0] "{n} letra asignada"
msgstr[1] "{n} letras asignadas"

msgid "App for “{}”"
msgstr "App para la «{}»"
"#;

    #[test]
    fn singular_and_multiline() {
        let c = Catalog::parse(PO);
        assert_eq!(c.tr("Save"), "Guardar");
        assert_eq!(c.tr("Search an app or a letter…"), "Buscar app o letra…");
        assert_eq!(c.tr("App for “{}”"), "App para la «{}»");
        assert_eq!(c.tr("Missing"), "Missing");
    }

    #[test]
    fn plural_forms() {
        let c = Catalog::parse(PO);
        assert_eq!(c.trn("{n} letter assigned", "{n} letters assigned", 1), "1 letra asignada");
        assert_eq!(c.trn("{n} letter assigned", "{n} letters assigned", 12), "12 letras asignadas");
        let empty = Catalog::default();
        let zero = empty.trn("{n} letter assigned", "{n} letters assigned", 0);
        assert_eq!(zero, "0 letters assigned");
    }

    #[test]
    fn the_settings_catalog_parses() {
        let po = include_str!("../../../settings/lang/es/LC_MESSAGES/wm-settings.po");
        let c = Catalog::parse(po);
        assert_eq!(c.tr("Apps and keys"), "Apps y teclas");
        assert_eq!(c.trn("{n} letter assigned", "{n} letters assigned", 2), "2 letras asignadas");
    }
}
