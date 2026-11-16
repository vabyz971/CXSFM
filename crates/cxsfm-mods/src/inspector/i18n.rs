//! inspector strings, owned by the mod.

/// Tile label for `lang_code` (unknown falls back to English).
pub fn tile_label(lang_code: &str) -> &'static str {
    match lang_code {
        "fr" => "Inspecteur",
        "de" => "Inspektor",
        "es" => "Inspector",
        "it" => "Ispettore",
        "pt" => "Inspetor",
        "ru" => "Инспектор",
        _ => "Inspector",
    }
}

/// One-line description for the About page.
pub fn describe(lang_code: &str) -> &'static str {
    match lang_code {
        "fr" => "Arbre de la scène : noms, ID et valeurs (lecture seule).",
        "de" => "Szenenbaum: Namen, IDs und Werte (nur Lesen).",
        "es" => "Árbol de la escena: nombres, ID y valores (lectura).",
        "it" => "Albero della scena: nomi, ID e valori (lettura).",
        "pt" => "Árvore da cena: nomes, ID e valores (leitura).",
        "ru" => "Дерево сцены: имена, ID и значения (чтение).",
        _ => "Scene tree: names, IDs and values (read-only).",
    }
}
