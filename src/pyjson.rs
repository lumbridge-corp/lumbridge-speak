//! JSON written the way Python's `json.dumps` writes it by default: `", "` between
//! items, `": "` after keys, and everything outside ASCII as `\uXXXX`.
//!
//! The Python front answered like this, and the model manager's health check looks for
//! the exact text `"ok": true`, so the spacing is part of the API.

use std::io;

use serde::Serialize;
use serde_json::ser::Formatter;

struct Python;

impl Formatter for Python {
    fn begin_array_value<W: ?Sized + io::Write>(&mut self, w: &mut W, first: bool) -> io::Result<()> {
        if first { Ok(()) } else { w.write_all(b", ") }
    }

    fn begin_object_key<W: ?Sized + io::Write>(&mut self, w: &mut W, first: bool) -> io::Result<()> {
        if first { Ok(()) } else { w.write_all(b", ") }
    }

    fn begin_object_value<W: ?Sized + io::Write>(&mut self, w: &mut W) -> io::Result<()> {
        w.write_all(b": ")
    }

    fn write_string_fragment<W: ?Sized + io::Write>(&mut self, w: &mut W, fragment: &str) -> io::Result<()> {
        for c in fragment.chars() {
            if c.is_ascii() {
                w.write_all(&[c as u8])?;
            } else {
                let mut units = [0u16; 2];
                for unit in c.encode_utf16(&mut units) {
                    write!(w, "\\u{unit:04x}")?;
                }
            }
        }
        Ok(())
    }
}

pub fn to_vec<T: Serialize>(value: &T) -> Vec<u8> {
    let mut out = Vec::new();
    let mut ser = serde_json::Serializer::with_formatter(&mut out, Python);
    value.serialize(&mut ser).expect("JSON values always serialize");
    out
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    #[test]
    fn matches_python_json_dumps() {
        let v = json!({"ok": true, "n": [1, 2.5, null], "s": "caf\u{e9} \u{1F600} \"q\"\n", "o": {}});
        // python3 -c 'import json; print(json.dumps({"ok": True, "n": [1, 2.5, None], "s": "café 😀 \"q\"\n", "o": {}}))'
        let want = r#"{"ok": true, "n": [1, 2.5, null], "s": "caf\u00e9 \ud83d\ude00 \"q\"\n", "o": {}}"#;
        assert_eq!(String::from_utf8(super::to_vec(&v)).unwrap(), want);
    }
}
