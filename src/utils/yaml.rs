//! Lecteur YAML pour les fichiers de plugins (`plugin.yml`, configurations).
//!
//! Il couvre le sous-ensemble utile à une configuration : sections imbriquées par
//! indentation, listes (`- élément` ou `[a, b]`), textes entre guillemets ou non,
//! nombres, booléens, `null` et commentaires. Les ancres, alias, textes sur plusieurs
//! lignes et documents multiples de la norme complète ne sont pas pris en charge.
//! Le résultat utilise le même type `Value` que le lecteur JSON.

use crate::json::Value;

const MAX_BYTES: usize = 256 * 1024;
const MAX_LINES: usize = 5000;
const MAX_DEPTH: usize = 32;

struct Line {
    indent: usize,
    text: String,
    number: usize,
}

/// Retire un commentaire (`#` en début de ligne ou après un espace, hors guillemets).
fn strip_comment(line: &str) -> &str {
    let mut quote: Option<char> = None;
    let mut prev = ' ';
    for (i, c) in line.char_indices() {
        match quote {
            Some(q) => {
                if c == q && !(q == '"' && prev == '\\') {
                    quote = None;
                }
            }
            None => {
                if c == '"' || c == '\'' {
                    quote = Some(c);
                } else if c == '#' && prev.is_whitespace() {
                    return &line[..i];
                }
            }
        }
        prev = c;
    }
    line
}

/// Texte entre guillemets, sans eux ; None s'il n'est pas entre guillemets.
fn unquote(text: &str) -> Option<String> {
    let t = text.trim();
    let bytes = t.as_bytes();
    if t.len() >= 2 && bytes[0] == b'"' && bytes[t.len() - 1] == b'"' {
        let mut out = String::new();
        let mut chars = t[1..t.len() - 1].chars();
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
        return Some(out);
    }
    if t.len() >= 2 && bytes[0] == b'\'' && bytes[t.len() - 1] == b'\'' {
        return Some(t[1..t.len() - 1].replace("''", "'"));
    }
    None
}

/// Découpe `a, "b, c", d` aux virgules situées hors des guillemets et des crochets.
fn split_flow(text: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let (mut depth, mut start) = (0i32, 0usize);
    let mut quote: Option<char> = None;
    for (i, c) in text.char_indices() {
        match quote {
            Some(q) => {
                if c == q {
                    quote = None;
                }
            }
            None => match c {
                '"' | '\'' => quote = Some(c),
                '[' | '{' => depth += 1,
                ']' | '}' => depth -= 1,
                ',' if depth == 0 => {
                    parts.push(&text[start..i]);
                    start = i + 1;
                }
                _ => {}
            },
        }
    }
    if !text[start..].trim().is_empty() {
        parts.push(&text[start..]);
    }
    parts
}

/// Sépare `clé: valeur` ; la clé peut être entre guillemets.
fn split_key(text: &str) -> Option<(String, &str)> {
    let mut quote: Option<char> = None;
    let bytes = text.as_bytes();
    for (i, c) in text.char_indices() {
        match quote {
            Some(q) => {
                if c == q {
                    quote = None;
                }
            }
            None => {
                if c == '"' || c == '\'' {
                    quote = Some(c);
                } else if c == ':' && (i + 1 == text.len() || bytes[i + 1] == b' ') {
                    let raw = text[..i].trim();
                    let key = unquote(raw).unwrap_or_else(|| raw.to_string());
                    if key.is_empty() {
                        return None;
                    }
                    return Some((key, text[i + 1..].trim()));
                }
            }
        }
    }
    None
}

/// Valeur simple : texte, nombre, booléen, ou liste entre crochets.
fn scalar(text: &str, depth: usize) -> Result<Value, String> {
    let t = text.trim();
    if depth > MAX_DEPTH {
        return Err("imbrication trop profonde".into());
    }
    if let Some(s) = unquote(t) {
        return Ok(Value::Str(s));
    }
    if t.starts_with('[') && t.ends_with(']') {
        let mut items = Vec::new();
        for part in split_flow(&t[1..t.len() - 1]) {
            items.push(scalar(part, depth + 1)?);
        }
        return Ok(Value::Arr(items));
    }
    if t.starts_with('{') && t.ends_with('}') {
        let mut items = Vec::new();
        for part in split_flow(&t[1..t.len() - 1]) {
            let (key, value) = split_key(part.trim()).ok_or("section entre accolades mal formée")?;
            items.push((key, scalar(value, depth + 1)?));
        }
        return Ok(Value::Obj(items));
    }
    Ok(match t {
        "" | "~" | "null" => Value::Null,
        "true" | "True" => Value::Bool(true),
        "false" | "False" => Value::Bool(false),
        _ => match t.parse::<f64>() {
            // Un nombre seulement s'il se réécrit à l'identique : « 1.0 » reste un texte (version)
            Ok(n) if n.is_finite() && format_number(n) == t => Value::Num(n),
            _ => Value::Str(t.to_string()),
        },
    })
}

/// Écriture d'un nombre : entier sans décimales quand c'en est un.
pub fn format_number(n: f64) -> String {
    if n.fract() == 0.0 && n.abs() < 1e15 {
        format!("{}", n as i64)
    } else {
        format!("{}", n)
    }
}

/// Bloc indenté : liste de « - » ou suite de « clé: valeur ».
fn block(lines: &mut Vec<Line>, pos: &mut usize, indent: usize, depth: usize) -> Result<Value, String> {
    if depth > MAX_DEPTH {
        return Err("imbrication trop profonde".into());
    }
    let is_item = |t: &str| t == "-" || t.starts_with("- ");
    if is_item(&lines[*pos].text) {
        let mut items = Vec::new();
        while *pos < lines.len() && lines[*pos].indent == indent && is_item(&lines[*pos].text) {
            let rest = lines[*pos].text[1..].trim().to_string();
            if rest.is_empty() {
                *pos += 1;
                if *pos < lines.len() && lines[*pos].indent > indent {
                    let inner = lines[*pos].indent;
                    items.push(block(lines, pos, inner, depth + 1)?);
                } else {
                    items.push(Value::Null);
                }
            } else if split_key(&rest).is_some() && !rest.starts_with('[') && !rest.starts_with('{') && unquote(&rest).is_none() {
                // « - clé: valeur » : l'élément est une section qui commence sur cette ligne
                lines[*pos].indent = indent + 2;
                lines[*pos].text = rest;
                items.push(block(lines, pos, indent + 2, depth + 1)?);
            } else {
                items.push(scalar(&rest, depth)?);
                *pos += 1;
            }
        }
        return Ok(Value::Arr(items));
    }
    let mut items: Vec<(String, Value)> = Vec::new();
    while *pos < lines.len() && lines[*pos].indent == indent {
        let number = lines[*pos].number;
        let (key, value) = match split_key(&lines[*pos].text) {
            Some((k, v)) => (k, v.to_string()),
            None => return Err(format!("ligne {} : « clé: valeur » attendu", number)),
        };
        *pos += 1;
        let parsed = if value.is_empty() {
            if *pos < lines.len() && lines[*pos].indent > indent {
                let inner = lines[*pos].indent;
                block(lines, pos, inner, depth + 1)?
            } else if *pos < lines.len() && lines[*pos].indent == indent && is_item(&lines[*pos].text) {
                block(lines, pos, indent, depth + 1)?
            } else {
                Value::Null
            }
        } else {
            scalar(&value, depth).map_err(|e| format!("ligne {} : {}", number, e))?
        };
        if items.iter().any(|(k, _)| *k == key) {
            return Err(format!("ligne {} : la clé « {} » apparaît deux fois", number, key));
        }
        items.push((key, parsed));
    }
    if *pos < lines.len() && lines[*pos].indent > indent {
        return Err(format!("ligne {} : indentation inattendue", lines[*pos].number));
    }
    Ok(Value::Obj(items))
}

/// Lit un document YAML.
pub fn parse(text: &str) -> Result<Value, String> {
    if text.len() > MAX_BYTES {
        return Err("fichier trop volumineux".into());
    }
    let mut lines = Vec::new();
    for (i, raw) in text.lines().enumerate() {
        let raw = raw.trim_end_matches('\r').trim_start_matches('\u{feff}');
        let content = strip_comment(raw).trim_end();
        let trimmed = content.trim_start();
        if trimmed.is_empty() || trimmed == "---" {
            continue;
        }
        let lead = &content[..content.len() - trimmed.len()];
        if lead.contains('\t') {
            return Err(format!("ligne {} : indentez avec des espaces, pas des tabulations", i + 1));
        }
        lines.push(Line { indent: lead.len(), text: trimmed.to_string(), number: i + 1 });
        if lines.len() > MAX_LINES {
            return Err("fichier trop long".into());
        }
    }
    if lines.is_empty() {
        return Ok(Value::Obj(Vec::new()));
    }
    let mut pos = 0;
    let indent = lines[0].indent;
    let value = block(&mut lines, &mut pos, indent, 0)?;
    if pos < lines.len() {
        return Err(format!("ligne {} : indentation incohérente", lines[pos].number));
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_plugin_manifest_is_read() {
        let v = parse(
            "# Mon plugin\nname: Gardien\nversion: 1.0\napi: 1\nmain: main.lua   # fichier principal\nauthors: [Enzo, \"Jean, dit J\"]\ndepend:\n  - Economie\n  - Monde\ncommands:\n  expulser:\n    description: 'Expulse un joueur : vite'\n    permission: gardien.expulser\npermissions:\n  gardien.expulser:\n    default: op\n  gardien.voir:\n    default: true\nvide:\n",
        )
        .unwrap();
        assert_eq!(v.get("name").and_then(|x| x.as_str()), Some("Gardien"));
        // « 1.0 » reste un texte ; « 1 » est un nombre
        assert_eq!(v.get("version"), Some(&Value::Str("1.0".into())));
        assert_eq!(v.get("api"), Some(&Value::Num(1.0)));
        assert_eq!(v.get("main").and_then(|x| x.as_str()), Some("main.lua"));
        assert_eq!(v.get("authors"), Some(&Value::Arr(vec![Value::Str("Enzo".into()), Value::Str("Jean, dit J".into())])));
        assert_eq!(v.get("depend").and_then(|d| d.as_arr()).map(|a| a.len()), Some(2));
        let cmd = v.get("commands").and_then(|c| c.get("expulser")).unwrap();
        assert_eq!(cmd.get("description").and_then(|x| x.as_str()), Some("Expulse un joueur : vite"));
        let perms = v.get("permissions").unwrap();
        assert_eq!(perms.get("gardien.expulser").and_then(|p| p.get("default")), Some(&Value::Str("op".into())));
        assert_eq!(perms.get("gardien.voir").and_then(|p| p.get("default")), Some(&Value::Bool(true)));
        assert_eq!(v.get("vide"), Some(&Value::Null));
    }

    #[test]
    fn lists_of_sections_and_same_level_lists() {
        let v = parse("recompenses:\n- objet: diamant\n  quantite: 3\n- objet: fer\n  quantite: 10\noptions: {pvp: false, max: 20}\n").unwrap();
        let list = v.get("recompenses").and_then(|r| r.as_arr()).unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[1].get("quantite"), Some(&Value::Num(10.0)));
        assert_eq!(v.get("options").and_then(|o| o.get("pvp")), Some(&Value::Bool(false)));
    }

    #[test]
    fn mistakes_are_reported_with_their_line() {
        assert!(parse("a: 1\n\tb: 2\n").unwrap_err().contains("ligne 2"));
        assert!(parse("a: 1\nsans deux points\n").unwrap_err().contains("ligne 2"));
        assert!(parse("a: 1\na: 2\n").unwrap_err().contains("deux fois"));
        assert!(parse("a: 1\n    b: 2\n").unwrap_err().contains("ligne 2"));
        assert!(parse(&"a: [".repeat(1)).is_ok());
        assert!(parse(&format!("a: {}{}", "[".repeat(100), "]".repeat(100))).is_err());
        assert_eq!(parse("").unwrap(), Value::Obj(vec![]));
    }
}
