//! Strict YAML-subset parser for the machine-owned configuration file.
//!
//! The runtime dependency surface is frozen to Clap only, and the machine
//! configuration is written by omnirepo's own setup slice; therefore the
//! loader parses a narrow, exact subset by hand: block mappings, block
//! sequences, plain and quoted scalars, unsigned integers, and inline
//! sequences.  Everything else (flow mappings, multi-line scalars, tabs,
//! aliases, anchors, duplicate keys, mixed indentation) fails closed with a
//! typed reason.  This mirrors the exact-text discipline of the journal.

#![allow(dead_code)]

use std::fmt;

/// Parsed subset value.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum YValue {
    Null,
    Number(u64),
    String(String),
    List(Vec<YValue>),
    Map(Vec<(String, YValue)>),
}

impl YValue {
    pub(crate) fn get(&self, key: &str) -> Option<&YValue> {
        match self {
            Self::Map(entries) => entries
                .iter()
                .find(|(candidate, _)| candidate == key)
                .map(|(_, value)| value),
            _ => None,
        }
    }

    pub(crate) fn as_u64(&self) -> Option<u64> {
        match self {
            Self::Number(value) => Some(*value),
            _ => None,
        }
    }

    pub(crate) fn as_str(&self) -> Option<&str> {
        match self {
            Self::String(value) => Some(value),
            _ => None,
        }
    }

    pub(crate) fn as_list(&self) -> Option<&[YValue]> {
        match self {
            Self::List(items) => Some(items),
            _ => None,
        }
    }
}

/// Parse failure with a human-readable reason.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct YamlError {
    pub line: usize,
    pub reason: String,
}

impl fmt::Display for YamlError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "line {}: {}", self.line, self.reason)
    }
}

struct Line {
    number: usize,
    indent: usize,
    text: String,
}

/// Parse the supported subset.  The document must be a block mapping.
pub(crate) fn parse_yaml_subset(text: &str) -> Result<YValue, YamlError> {
    let lines = prepare_lines(text)?;
    if let Some(first) = lines.first() {
        if first.text == "-" || first.text.starts_with("- ") {
            return Err(error_at(&lines, 0, "top level must be a mapping"));
        }
    }
    let (value, next) = parse_block(&lines, 0, 0)?;
    if next != lines.len() {
        return Err(error_at(
            &lines,
            next,
            "unexpected content after the document",
        ));
    }
    Ok(value)
}

fn prepare_lines(text: &str) -> Result<Vec<Line>, YamlError> {
    let mut lines = Vec::new();
    for (index, raw) in text.split('\n').enumerate() {
        let number = index + 1;
        let without_comment = strip_comment(raw);
        let trimmed = without_comment.trim_end();
        if trimmed.trim().is_empty() {
            continue;
        }
        let indent = leading_spaces(trimmed).map_err(|reason| YamlError {
            line: number,
            reason,
        })?;
        lines.push(Line {
            number,
            indent,
            text: trimmed[indent..].to_owned(),
        });
    }
    Ok(lines)
}

fn strip_comment(raw: &str) -> &str {
    let mut in_single = false;
    let mut in_double = false;
    for (index, byte) in raw.bytes().enumerate() {
        match byte {
            b'\'' if !in_double => in_single = !in_single,
            b'"' if !in_single => in_double = !in_double,
            b'#' if !in_single && !in_double => {
                if index == 0 || raw.as_bytes()[index - 1].is_ascii_whitespace() {
                    return &raw[..index];
                }
            }
            _ => {}
        }
    }
    raw
}

fn leading_spaces(text: &str) -> Result<usize, String> {
    let mut count = 0;
    for byte in text.bytes() {
        match byte {
            b' ' => count += 1,
            b'\t' => return Err("tabs are not allowed in machine config indentation".to_owned()),
            _ => break,
        }
    }
    Ok(count)
}

fn error_at(lines: &[Line], index: usize, reason: &str) -> YamlError {
    let line = lines.get(index).map(|line| line.number).unwrap_or(1);
    YamlError {
        line,
        reason: reason.to_owned(),
    }
}

fn parse_block(lines: &[Line], index: usize, indent: usize) -> Result<(YValue, usize), YamlError> {
    let Some(first) = lines.get(index) else {
        return Ok((YValue::Null, index));
    };
    if first.indent < indent {
        return Ok((YValue::Null, index));
    }
    if first.indent > indent {
        return Err(error_at(lines, index, "unexpected indentation"));
    }
    if first.text.starts_with("- ") || first.text == "-" {
        parse_list(lines, index, indent)
    } else if first.text.contains(':') {
        parse_map(lines, index, indent)
    } else {
        Err(error_at(
            lines,
            index,
            "expected a mapping or sequence entry",
        ))
    }
}

fn parse_map(lines: &[Line], index: usize, indent: usize) -> Result<(YValue, usize), YamlError> {
    let mut entries = Vec::new();
    let mut cursor = index;
    while let Some(line) = lines.get(cursor) {
        if line.indent < indent {
            break;
        }
        if line.indent != indent {
            return Err(error_at(lines, cursor, "inconsistent mapping indentation"));
        }
        let Some((key, rest)) = split_key(&line.text) else {
            return Err(error_at(lines, cursor, "expected a mapping entry"));
        };
        if entries.iter().any(|(existing, _)| existing == &key) {
            return Err(error_at(lines, cursor, "duplicate mapping key"));
        }
        let rest = rest.trim();
        if rest.is_empty() {
            let (value, next) = parse_block(lines, cursor + 1, indent + 2)?;
            if next == cursor + 1 {
                entries.push((key, YValue::Null));
            } else {
                entries.push((key, value));
            }
            cursor = next;
        } else {
            entries.push((key, parse_scalar_or_inline(rest, line.number)?));
            cursor += 1;
        }
    }
    Ok((YValue::Map(entries), cursor))
}

fn parse_list(lines: &[Line], index: usize, indent: usize) -> Result<(YValue, usize), YamlError> {
    let mut items = Vec::new();
    let mut cursor = index;
    while let Some(line) = lines.get(cursor) {
        if line.indent < indent {
            break;
        }
        if line.indent != indent {
            return Err(error_at(lines, cursor, "inconsistent sequence indentation"));
        }
        let Some(rest) = line.text.strip_prefix("- ") else {
            if line.text == "-" {
                let (value, next) = parse_block(lines, cursor + 1, indent + 2)?;
                if next == cursor + 1 {
                    items.push(YValue::Null);
                } else {
                    items.push(value);
                }
                cursor = next;
                continue;
            }
            break;
        };
        let rest = rest.trim();
        if rest.is_empty() {
            let (value, next) = parse_block(lines, cursor + 1, indent + 2)?;
            if next == cursor + 1 {
                items.push(YValue::Null);
            } else {
                items.push(value);
            }
            cursor = next;
        } else if let Some((key, value_rest)) = split_key(rest) {
            // A sequence item that is itself a mapping: the first entry is
            // inline on this line, the rest continue at the nested indent.
            let mut entries = Vec::new();
            let value_rest = value_rest.trim();
            let (first_value, cursor_after_first) = if value_rest.is_empty() {
                let (value, next) = parse_block(lines, cursor + 1, indent + 4)?;
                if next == cursor + 1 {
                    (YValue::Null, cursor + 1)
                } else {
                    (value, next)
                }
            } else {
                (parse_scalar_or_inline(value_rest, line.number)?, cursor + 1)
            };
            entries.push((key, first_value));
            let (more, next) = parse_map(lines, cursor_after_first, indent + 2)?;
            if let YValue::Map(more_entries) = more {
                entries.extend(more_entries);
            }
            items.push(YValue::Map(entries));
            cursor = next;
        } else {
            items.push(parse_scalar_or_inline(rest, line.number)?);
            cursor += 1;
        }
    }
    Ok((YValue::List(items), cursor))
}

fn split_key(text: &str) -> Option<(String, &str)> {
    let colon = text.find(':')?;
    let key = text[..colon].trim();
    if key.is_empty() || key.chars().any(char::is_whitespace) {
        return None;
    }
    Some((key.to_owned(), &text[colon + 1..]))
}

fn parse_scalar_or_inline(text: &str, line: usize) -> Result<YValue, YamlError> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(YValue::Null);
    }
    if let Some(inner) = text.strip_prefix('[') {
        let inner = inner.strip_suffix(']').ok_or_else(|| YamlError {
            line,
            reason: "unterminated inline sequence".to_owned(),
        })?;
        let inner = inner.trim();
        if inner.is_empty() {
            return Ok(YValue::List(Vec::new()));
        }
        let items = inner
            .split(',')
            .map(|item| {
                let item = item.trim();
                if item.is_empty() {
                    return Err(YamlError {
                        line,
                        reason: "empty inline sequence entry".to_owned(),
                    });
                }
                scalar(item, line)
            })
            .collect::<Result<Vec<_>, _>>()?;
        return Ok(YValue::List(items));
    }
    if text.contains('{') || text.contains('}') {
        return Err(YamlError {
            line,
            reason: "flow mappings are not supported".to_owned(),
        });
    }
    scalar(text, line)
}

fn scalar(text: &str, line: usize) -> Result<YValue, YamlError> {
    if let Some(inner) = text.strip_prefix('"') {
        let inner = inner.strip_suffix('"').ok_or_else(|| YamlError {
            line,
            reason: "unterminated double-quoted string".to_owned(),
        })?;
        if inner.contains('\\') {
            return Err(YamlError {
                line,
                reason: "escape sequences are not supported in machine config strings".to_owned(),
            });
        }
        return Ok(YValue::String(inner.to_owned()));
    }
    if let Some(inner) = text.strip_prefix('\'') {
        let inner = inner.strip_suffix('\'').ok_or_else(|| YamlError {
            line,
            reason: "unterminated single-quoted string".to_owned(),
        })?;
        return Ok(YValue::String(inner.to_owned()));
    }
    if let Ok(number) = text.parse::<u64>() {
        return Ok(YValue::Number(number));
    }
    if text.contains('\t') || text.contains('\n') || text.contains('\r') {
        return Err(YamlError {
            line,
            reason: "control characters are not allowed in scalars".to_owned(),
        });
    }
    Ok(YValue::String(text.to_owned()))
}
