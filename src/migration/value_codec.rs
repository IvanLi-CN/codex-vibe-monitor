//! Ledger v12 stores valid SQLite text without discarding NUL. The escape is
//! injective and preserves UTF-8 byte order, including prefixes and empty text.
use std::borrow::Cow;

use anyhow::{Result, bail};
use serde_json::Value;

pub(crate) fn encode_text(text: &str) -> Cow<'_, str> {
    if !text.contains(['\0', '\u{1}']) {
        return Cow::Borrowed(text);
    }
    let mut encoded = String::with_capacity(text.len() + 1);
    for ch in text.chars() {
        match ch {
            '\0' => encoded.push_str("\u{1}\u{1}"),
            '\u{1}' => encoded.push_str("\u{1}\u{2}"),
            ch => encoded.push(ch),
        }
    }
    Cow::Owned(encoded)
}

pub(crate) fn decode_text(text: &str) -> Result<Cow<'_, str>> {
    if !text.contains('\u{1}') {
        return Ok(Cow::Borrowed(text));
    }
    let mut decoded = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(ch) = chars.next() {
        if ch != '\u{1}' {
            decoded.push(ch);
            continue;
        }
        match chars.next() {
            Some('\u{1}') => decoded.push('\0'),
            Some('\u{2}') => decoded.push('\u{1}'),
            _ => bail!("invalid PostgreSQL primary text escape"),
        }
    }
    Ok(Cow::Owned(decoded))
}

pub(crate) fn encoded_json(value: &Value) -> Value {
    let mut value = value.clone();
    encode_json(&mut value);
    value
}

pub(crate) fn encode_json(value: &mut Value) {
    match value {
        Value::String(text) => {
            if let Cow::Owned(encoded) = encode_text(text) {
                *text = encoded;
            }
        }
        Value::Array(values) => values.iter_mut().for_each(encode_json),
        Value::Object(values) => {
            *values = std::mem::take(values)
                .into_iter()
                .map(|(key, mut value)| {
                    encode_json(&mut value);
                    (encode_text(&key).into_owned(), value)
                })
                .collect();
        }
        _ => {}
    }
}

pub(crate) fn decode_json(value: &mut Value) -> Result<()> {
    match value {
        Value::String(text) => {
            if let Cow::Owned(decoded) = decode_text(text)? {
                *text = decoded;
            }
        }
        Value::Array(values) => {
            for value in values {
                decode_json(value)?;
            }
        }
        Value::Object(values) => {
            *values = std::mem::take(values)
                .into_iter()
                .map(|(key, mut value)| {
                    decode_json(&mut value)?;
                    Ok((decode_text(&key)?.into_owned(), value))
                })
                .collect::<Result<_>>()?;
        }
        _ => {}
    }
    Ok(())
}

pub(crate) fn encoded_key(key: &str) -> Result<String> {
    let mut key: Value = serde_json::from_str(key)?;
    // Object keys are source column identifiers, not encoded business values.
    if let Some(columns) = key.as_object_mut() {
        for value in columns.values_mut() {
            encode_json(value);
        }
    }
    Ok(serde_json::to_string(&key)?)
}

pub(crate) fn decode_row(mut row: Value) -> Result<Value> {
    if let Some(columns) = row.as_object_mut() {
        for value in columns.values_mut() {
            if let Value::String(text) = value
                && let Cow::Owned(decoded) = decode_text(text)?
            {
                *text = decoded;
            }
        }
    }
    Ok(row)
}

pub(crate) fn projection(columns: &[(String, String)], alias: &str) -> String {
    columns
        .iter()
        .map(|(name, kind)| {
            let column = format!("{alias}.{}", super::quote_identifier(name));
            if super::postgres_type_for_source_column(kind) == "BYTEA" {
                format!(
                    "translate(encode({column},'base64'),E'\\n\\r','') AS {}",
                    super::quote_identifier(name)
                )
            } else {
                column
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_codec_is_lossless_and_preserves_byte_order() {
        let mut inputs = vec![String::new(), "replacement\u{FFFD}".into(), "日本語".into()];
        for byte in 0..=127 {
            inputs.push(char::from(byte).to_string());
            inputs.push(format!("a{}suffix", char::from(byte)));
        }
        inputs.extend(["\0\u{1}".into(), "\u{1}\u{1}".into(), "\u{1}\u{2}".into()]);
        for input in &inputs {
            assert_eq!(decode_text(&encode_text(input)).unwrap(), input.as_str());
        }
        for a in &inputs {
            for b in &inputs {
                assert_eq!(
                    a.as_bytes().cmp(b.as_bytes()),
                    encode_text(a).as_bytes().cmp(encode_text(b).as_bytes())
                );
            }
        }
        assert!(decode_text("\u{1}").is_err());
        assert!(decode_text("\u{1}x").is_err());
    }

    #[test]
    fn json_codec_restores_keys_values_and_arrays() {
        let value = serde_json::json!({"key\0": ["value\0", "literal\u{1}\u{1}", null, 4]});
        let mut encoded = encoded_json(&value);
        assert_ne!(encoded, value);
        decode_json(&mut encoded).unwrap();
        assert_eq!(encoded, value);
    }
}
