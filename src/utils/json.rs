//! Petit analyseur JSON et décodage base64url, pour lire les jetons de connexion.

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    Arr(Vec<Value>),
    Obj(Vec<(String, Value)>),
}

impl Value {
    /// Accès typés : champ d'un objet, texte, tableau, nombre, booléen.
    pub fn get(&self, key: &str) -> Option<&Value> {
        match self {
            Value::Obj(items) => items.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::Str(s) => Some(s),
            _ => None,
        }
    }
    pub fn as_arr(&self) -> Option<&[Value]> {
        match self {
            Value::Arr(a) => Some(a),
            _ => None,
        }
    }
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Value::Num(n) if n.is_finite() => Some(*n),
            _ => None,
        }
    }
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Bool(b) => Some(*b),
            _ => None,
        }
    }
    /// Remplace (ou ajoute) un champ d'un objet.
    pub fn set(&mut self, key: &str, value: Value) {
        if let Value::Obj(items) = self {
            match items.iter_mut().find(|(k, _)| k == key) {
                Some(slot) => slot.1 = value,
                None => items.push((key.to_string(), value)),
            }
        }
    }
    /// Texte JSON de la valeur.
    pub fn to_json(&self) -> String {
        let mut out = String::new();
        self.write(&mut out);
        out
    }
    /// Écrit la valeur en JSON compact.
    fn write(&self, out: &mut String) {
        match self {
            Value::Null => out.push_str("null"),
            Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
            Value::Num(n) if n.is_finite() => {
                if n.fract() == 0.0 && n.abs() < 1e15 {
                    out.push_str(&format!("{}", *n as i64));
                } else {
                    out.push_str(&format!("{}", n));
                }
            }
            Value::Num(_) => out.push_str("null"),
            Value::Str(s) => write_str(s, out),
            Value::Arr(items) => {
                out.push('[');
                for (i, v) in items.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    v.write(out);
                }
                out.push(']');
            }
            Value::Obj(items) => {
                out.push('{');
                for (i, (k, v)) in items.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    write_str(k, out);
                    out.push(':');
                    v.write(out);
                }
                out.push('}');
            }
        }
    }
}

/// Texte JSON avec échappements.
fn write_str(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}

#[cfg(test)]
mod writer_tests {
    use super::*;

    #[test]
    fn values_survive_a_round_trip() {
        let v = Value::Obj(vec![
            ("name".into(), Value::Str("a \"b\"\n\u{1}é".into())),
            ("n".into(), Value::Num(3.0)),
            ("x".into(), Value::Num(-0.5)),
            ("ok".into(), Value::Bool(true)),
            ("list".into(), Value::Arr(vec![Value::Null, Value::Arr(vec![])])),
        ]);
        let text = v.to_json();
        assert!(text.contains("\"n\":3,"));
        assert_eq!(parse(&text).unwrap(), v);
    }
}

struct Parser<'a> {
    s: &'a [u8],
    pos: usize,
}

pub fn parse(text: &str) -> Result<Value, &'static str> {
    let mut p = Parser { s: text.as_bytes(), pos: 0 };
    let v = p.value(0)?;
    p.ws();
    if p.pos != p.s.len() {
        return Err("JSON : données en trop");
    }
    Ok(v)
}

impl<'a> Parser<'a> {
    fn ws(&mut self) {
        while let Some(b) = self.s.get(self.pos) {
            if matches!(b, b' ' | b'\t' | b'\n' | b'\r') {
                self.pos += 1;
            } else {
                break;
            }
        }
    }

    fn lit(&mut self, word: &str, v: Value) -> Result<Value, &'static str> {
        if self.s[self.pos..].starts_with(word.as_bytes()) {
            self.pos += word.len();
            Ok(v)
        } else {
            Err("JSON : valeur invalide")
        }
    }

    fn value(&mut self, depth: u32) -> Result<Value, &'static str> {
        if depth > 32 {
            return Err("JSON : imbrication trop profonde");
        }
        self.ws();
        match self.s.get(self.pos).copied() {
            Some(b'{') => {
                self.pos += 1;
                let mut items = Vec::new();
                self.ws();
                if self.s.get(self.pos) == Some(&b'}') {
                    self.pos += 1;
                    return Ok(Value::Obj(items));
                }
                loop {
                    self.ws();
                    let k = self.string()?;
                    self.ws();
                    if self.s.get(self.pos) != Some(&b':') {
                        return Err("JSON : ':' attendu");
                    }
                    self.pos += 1;
                    let v = self.value(depth + 1)?;
                    items.push((k, v));
                    self.ws();
                    match self.s.get(self.pos) {
                        Some(b',') => self.pos += 1,
                        Some(b'}') => {
                            self.pos += 1;
                            return Ok(Value::Obj(items));
                        }
                        _ => return Err("JSON : objet mal formé"),
                    }
                }
            }
            Some(b'[') => {
                self.pos += 1;
                let mut items = Vec::new();
                self.ws();
                if self.s.get(self.pos) == Some(&b']') {
                    self.pos += 1;
                    return Ok(Value::Arr(items));
                }
                loop {
                    items.push(self.value(depth + 1)?);
                    self.ws();
                    match self.s.get(self.pos) {
                        Some(b',') => self.pos += 1,
                        Some(b']') => {
                            self.pos += 1;
                            return Ok(Value::Arr(items));
                        }
                        _ => return Err("JSON : tableau mal formé"),
                    }
                }
            }
            Some(b'"') => Ok(Value::Str(self.string()?)),
            Some(b't') => self.lit("true", Value::Bool(true)),
            Some(b'f') => self.lit("false", Value::Bool(false)),
            Some(b'n') => self.lit("null", Value::Null),
            Some(_) => {
                let start = self.pos;
                while let Some(b) = self.s.get(self.pos) {
                    if matches!(b, b'0'..=b'9' | b'-' | b'+' | b'.' | b'e' | b'E') {
                        self.pos += 1;
                    } else {
                        break;
                    }
                }
                std::str::from_utf8(&self.s[start..self.pos])
                    .ok()
                    .and_then(|t| t.parse().ok())
                    .map(Value::Num)
                    .ok_or("JSON : nombre invalide")
            }
            None => Err("JSON : fin inattendue"),
        }
    }

    fn string(&mut self) -> Result<String, &'static str> {
        if self.s.get(self.pos) != Some(&b'"') {
            return Err("JSON : chaîne attendue");
        }
        self.pos += 1;
        let mut out: Vec<u8> = Vec::new();
        loop {
            let b = *self.s.get(self.pos).ok_or("JSON : chaîne non terminée")?;
            self.pos += 1;
            match b {
                b'"' => break,
                b'\\' => {
                    let e = *self.s.get(self.pos).ok_or("JSON : chaîne non terminée")?;
                    self.pos += 1;
                    let c = match e {
                        b'n' => '\n',
                        b't' => '\t',
                        b'r' => '\r',
                        b'b' => '\u{8}',
                        b'f' => '\u{c}',
                        b'u' => {
                            let hex = self.s.get(self.pos..self.pos + 4).ok_or("JSON : \\u invalide")?;
                            self.pos += 4;
                            let n = std::str::from_utf8(hex)
                                .ok()
                                .and_then(|h| u32::from_str_radix(h, 16).ok())
                                .ok_or("JSON : \\u invalide")?;
                            char::from_u32(n).unwrap_or('\u{fffd}')
                        }
                        other => other as char,
                    };
                    let mut tmp = [0u8; 4];
                    out.extend_from_slice(c.encode_utf8(&mut tmp).as_bytes());
                }
                _ => out.push(b),
            }
        }
        String::from_utf8(out).map_err(|_| "JSON : UTF-8 invalide")
    }
}

/// Décode du base64 (alphabet standard ou « url »), sans exiger le remplissage.
pub fn base64_decode(text: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(text.len() * 3 / 4);
    let (mut acc, mut bits) = (0u32, 0u32);
    for c in text.bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'-' | b'+' => 62,
            b'_' | b'/' => 63,
            b'=' => break,
            _ => return None,
        };
        acc = (acc << 6) | v as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    Some(out)
}

/// Encode en base64, alphabet standard ou « url », avec ou sans remplissage.
pub fn base64_encode(data: &[u8], url_safe: bool, pad: bool) -> String {
    let table: &[u8; 64] = if url_safe {
        b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_"
    } else {
        b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/"
    };
    let mut out = String::with_capacity((data.len() + 2) / 3 * 4);
    for c in data.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        out.push(table[(n >> 18) as usize & 63] as char);
        out.push(table[(n >> 12) as usize & 63] as char);
        if c.len() > 1 {
            out.push(table[(n >> 6) as usize & 63] as char);
        } else if pad {
            out.push('=');
        }
        if c.len() > 2 {
            out.push(table[n as usize & 63] as char);
        } else if pad {
            out.push('=');
        }
    }
    out
}

/// Renvoie le contenu (non vérifié) d'un jeton JWT.
pub fn jwt_payload(jwt: &str) -> Option<Value> {
    let part = jwt.split('.').nth(1)?;
    let bytes = base64_decode(part)?;
    parse(std::str::from_utf8(&bytes).ok()?).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_nested() {
        let v = parse(r#"{"a":[1,2,{"b":"c\né"}],"d":true,"e":null}"#).unwrap();
        assert_eq!(v.get("a").unwrap().as_arr().unwrap()[2].get("b").unwrap().as_str(), Some("c\né"));
        assert_eq!(v.get("d"), Some(&Value::Bool(true)));
    }

    #[test]
    fn rejects_garbage() {
        assert!(parse("{").is_err());
        assert!(parse("[1,]").is_err());
        assert!(parse(&"[".repeat(1000)).is_err());
    }

    #[test]
    fn base64_roundtrip() {
        for len in 0..40usize {
            let data: Vec<u8> = (0..len).map(|i| (i * 37 + 11) as u8).collect();
            for (url, pad) in [(false, true), (true, false), (false, false)] {
                assert_eq!(base64_decode(&base64_encode(&data, url, pad)).unwrap(), data);
            }
        }
        assert_eq!(base64_encode(b"hello", false, true), "aGVsbG8=");
        assert_eq!(base64_encode(&[0xfb, 0xff], true, false), "-_8");
    }

    #[test]
    fn base64() {
        assert_eq!(base64_decode("aGVsbG8").unwrap(), b"hello");
        assert_eq!(base64_decode("aGVsbG8=").unwrap(), b"hello");
    }
}
