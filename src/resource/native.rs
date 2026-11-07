//! Read-only native snapshots. Decode failures are unavailable observations.

use anyhow::{Context, Result, bail};
use plist::{Dictionary, Value};
use quick_xml::events::Event;

const MAX_BYTES: usize = 8 * 1024 * 1024;
const MAX_DEPTH: usize = 64;
const MAX_EVENTS: usize = 100_000;

pub(super) fn identifier(value: &str, label: &str) -> Result<()> {
    if value.is_empty() || value.starts_with('-') || value.chars().any(char::is_control) {
        bail!("Invalid {label}; no command was scheduled");
    }
    Ok(())
}

pub(super) fn preferences(domain: &str) -> Result<Dictionary> {
    identifier(domain, "preference domain")?;
    parse_preferences(&crate::runner::observe(
        "defaults",
        ["export", domain, "-"],
    )?)
}

fn parse_preferences(bytes: &[u8]) -> Result<Dictionary> {
    if bytes.len() > MAX_BYTES {
        bail!("Native preference snapshot exceeds byte limit");
    }
    let mut reader = quick_xml::Reader::from_reader(bytes);
    let mut depth = 0_usize;
    let mut count = 0_usize;
    let mut dictionaries: Vec<(usize, std::collections::HashSet<String>)> = Vec::new();
    let mut key: Option<String> = None;
    loop {
        count += 1;
        if count > MAX_EVENTS {
            bail!("Native preference snapshot exceeds event limit");
        }
        match reader
            .read_event()
            .context("Invalid native preference XML")?
        {
            Event::Start(tag) => {
                if key.is_some() {
                    bail!("Nested content in native preference key");
                }
                depth += 1;
                if depth > MAX_DEPTH {
                    bail!("Native preference snapshot exceeds nesting limit");
                }
                match tag.name().as_ref() {
                    b"dict" => dictionaries.push((depth, std::collections::HashSet::new())),
                    b"key" => {
                        if dictionaries
                            .last()
                            .is_none_or(|(parent, _)| *parent + 1 != depth)
                        {
                            bail!("Native preference key is outside a dictionary");
                        }
                        key = Some(String::new());
                    }
                    _ => {}
                }
            }
            Event::End(tag) => {
                match tag.name().as_ref() {
                    b"key" => {
                        let value = key.take().context("Invalid native preference key")?;
                        if !dictionaries
                            .last_mut()
                            .context("Missing native dictionary")?
                            .1
                            .insert(value)
                        {
                            bail!("Duplicate native preference key");
                        }
                    }
                    b"dict" => {
                        dictionaries
                            .pop()
                            .context("Invalid native dictionary nesting")?;
                    }
                    _ => {}
                }
                depth = depth
                    .checked_sub(1)
                    .context("Invalid native preference nesting")?;
            }
            Event::Empty(_) if depth >= MAX_DEPTH => {
                bail!("Native preference snapshot exceeds nesting limit")
            }
            Event::Empty(tag) if tag.name().as_ref() == b"key" => {
                let (parent, keys) = dictionaries
                    .last_mut()
                    .context("Native key is outside a dictionary")?;
                if *parent != depth || !keys.insert(String::new()) {
                    bail!("Invalid or duplicate empty native preference key");
                }
            }
            Event::Text(text) => {
                if let Some(key) = &mut key {
                    key.push_str(&text.xml_content().context("Invalid native key text")?);
                }
            }
            Event::CData(text) => {
                if let Some(key) = &mut key {
                    key.push_str(&text.decode().context("Invalid native key text")?);
                }
            }
            Event::GeneralRef(reference) => {
                if let Some(key) = &mut key {
                    if let Some(value) = reference
                        .resolve_char_ref()
                        .context("Invalid native key reference")?
                    {
                        key.push(value);
                    } else {
                        let name = reference.decode().context("Invalid native key reference")?;
                        key.push_str(
                            quick_xml::escape::resolve_predefined_entity(&name)
                                .context("Unsupported native key entity")?,
                        );
                    }
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    if depth != 0 {
        bail!("Incomplete native preference XML");
    }
    // Only bounded XML goes through the recursive value builder. Neither this
    // validator nor plist's reader resolves the external Apple DTD over network.
    let value =
        Value::from_reader_xml(bytes).context("Could not decode native preference snapshot")?;
    value
        .into_dictionary()
        .context("Native preferences must be a dictionary")
}

pub(super) fn package_name(value: &str, label: &str) -> Result<()> {
    identifier(value, label)?;
    if value.chars().any(char::is_whitespace) {
        bail!("Invalid {label}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_preferences_preserve_native_types_and_missing_keys() {
        let parsed = parse_preferences(b"<plist version='1.0'><dict><key>flag</key><true/><key>text</key><string>1 </string></dict></plist>").unwrap();
        assert_eq!(parsed["flag"].as_boolean(), Some(true));
        assert_eq!(parsed["text"].as_string(), Some("1 "));
        assert!(parsed.get("missing").is_none());
        assert!(parse_preferences(b"not XML").is_err());
        assert!(parse_preferences(b"<plist><array/></plist>").is_err());
        for invalid in [
            b"<plist><dict/></plist><plist><dict/></plist>".as_slice(),
            b"<plist><dict/><dict/></plist>",
            b"<plist><dict/></plist>garbage",
        ] {
            assert!(parse_preferences(invalid).is_err());
        }
    }

    #[test]
    fn deep_preferences_fail_before_recursive_value_building() {
        let nested = format!(
            "<plist><dict><key>nested</key>{}<string>x</string>{}</dict></plist>",
            "<array>".repeat(256),
            "</array>".repeat(256)
        );
        assert!(
            parse_preferences(nested.as_bytes())
                .unwrap_err()
                .to_string()
                .contains("nesting limit")
        );
        let shallow = format!(
            "<plist><dict><key>large</key><string>{}</string></dict></plist>",
            "x".repeat(1024 * 1024)
        );
        assert_eq!(
            parse_preferences(shallow.as_bytes()).unwrap()["large"]
                .as_string()
                .unwrap()
                .len(),
            1024 * 1024
        );
    }

    #[test]
    fn duplicate_keys_are_unavailable_even_after_entity_decoding() {
        for key in ["key", "k&#101;y", "k<![CDATA[e]]>y"] {
            let xml = format!(
                "<plist><dict><key>key</key><true/><key>{key}</key><false/></dict></plist>"
            );
            assert!(
                parse_preferences(xml.as_bytes())
                    .unwrap_err()
                    .to_string()
                    .contains("Duplicate")
            );
        }
        let nested = b"<plist><dict><key>outer</key><dict><key>x</key><true/><key>x</key><false/></dict></dict></plist>";
        assert!(parse_preferences(nested).is_err());
    }

    #[test]
    fn byte_event_and_exact_nesting_limits_are_enforced() {
        assert!(
            parse_preferences(&vec![b'x'; MAX_BYTES + 1])
                .unwrap_err()
                .to_string()
                .contains("byte limit")
        );
        let events = format!(
            "<plist><dict><key>x</key><array>{}</array></dict></plist>",
            "<true/>".repeat(MAX_EVENTS)
        );
        assert!(
            parse_preferences(events.as_bytes())
                .unwrap_err()
                .to_string()
                .contains("event limit")
        );
        for (arrays, accepted) in [(61, true), (62, false)] {
            let xml = format!(
                "<plist><dict><key>x</key>{}<string/>{}</dict></plist>",
                "<array>".repeat(arrays),
                "</array>".repeat(arrays)
            );
            assert_eq!(parse_preferences(xml.as_bytes()).is_ok(), accepted);
        }
    }
}
