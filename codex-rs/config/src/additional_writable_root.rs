use std::collections::HashSet;
use std::io;
use std::path::PathBuf;

use codex_utils_absolute_path::AbsolutePathBuf;
use regex_lite::Regex;
use schemars::JsonSchema;
use schemars::SchemaGenerator;
use schemars::schema::Schema;
use serde::Deserialize;
use serde::Deserializer;
use serde::Serialize;
use serde::Serializer;

/// An explicit writable path or a vi-style substitution that derives paths.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AdditionalWritableRoot {
    Path(AbsolutePathBuf),
    Rewrite(String),
}

impl AdditionalWritableRoot {
    pub fn path(&self) -> Option<&AbsolutePathBuf> {
        match self {
            Self::Path(path) => Some(path),
            Self::Rewrite(_) => None,
        }
    }

    pub fn rewrite(&self) -> Option<&str> {
        match self {
            Self::Path(_) => None,
            Self::Rewrite(rule) => Some(rule),
        }
    }
}

impl<'de> Deserialize<'de> for AdditionalWritableRoot {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        if value.starts_with("s/") {
            Ok(Self::Rewrite(value))
        } else {
            AbsolutePathBuf::try_from(PathBuf::from(&value))
                .map(Self::Path)
                .map_err(serde::de::Error::custom)
        }
    }
}

impl Serialize for AdditionalWritableRoot {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            Self::Path(path) => serializer.serialize_str(&path.as_path().to_string_lossy()),
            Self::Rewrite(rule) => serializer.serialize_str(rule),
        }
    }
}

impl JsonSchema for AdditionalWritableRoot {
    fn schema_name() -> String {
        "AdditionalWritableRoot".to_owned()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        String::json_schema(generator)
    }
}

/// Applies vi-style substitutions to the original roots and returns unique derived roots.
///
/// Derived roots are never used as inputs, so entries cannot cause chaining.
pub fn derive_additional_writable_roots(
    entries: &[AdditionalWritableRoot],
    roots: &[AbsolutePathBuf],
) -> io::Result<Vec<AbsolutePathBuf>> {
    let compiled_rules = entries
        .iter()
        .enumerate()
        .filter_map(|(index, entry)| {
            entry.rewrite().map(|rule| {
                parse_substitution(rule).and_then(|(pattern, replacement)| {
                    Regex::new(&pattern)
                        .map(|regex| (index, regex, replacement))
                        .map_err(|error| {
                            io::Error::new(
                                io::ErrorKind::InvalidInput,
                                format!(
                                    "invalid additional writable root entry {index} regex: {error}"
                                ),
                            )
                        })
                })
            })
        })
        .collect::<io::Result<Vec<_>>>()?;

    let mut derived = Vec::new();
    let mut seen = HashSet::new();
    for (index, regex, replacement) in compiled_rules {
        for root in roots {
            let Some(path_text) = root.as_path().to_str() else {
                continue;
            };
            let Some(captures) = regex.captures(path_text) else {
                continue;
            };
            let matched = captures
                .get(0)
                .expect("regex captures always include the full match");
            let mut rewritten = String::with_capacity(path_text.len() + replacement.len());
            rewritten.push_str(&path_text[..matched.start()]);
            captures.expand(&replacement, &mut rewritten);
            rewritten.push_str(&path_text[matched.end()..]);

            let path = AbsolutePathBuf::from_absolute_path_checked(PathBuf::from(&rewritten))
                .map_err(|error| {
                    io::Error::new(
                        io::ErrorKind::InvalidInput,
                        format!(
                            "additional writable root entry {index} produced an invalid path `{rewritten}`: {error}"
                        ),
                    )
                })?;
            if seen.insert(path.clone()) {
                derived.push(path);
            }
        }
    }
    Ok(derived)
}

fn parse_substitution(rule: &str) -> io::Result<(String, String)> {
    let invalid = || {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "additional writable root entry must use s/pattern/replacement syntax",
        )
    };
    let body = rule.strip_prefix("s/").ok_or_else(invalid)?;
    let mut fields = [String::new(), String::new()];
    let mut field = 0;
    let mut chars = body.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\\' && chars.peek() == Some(&'/') {
            chars.next();
            fields[field].push('/');
        } else if ch == '/' {
            if field != 0 {
                return Err(invalid());
            }
            field = 1;
        } else {
            fields[field].push(ch);
        }
    }
    if field != 1 || fields[0].is_empty() || fields[1].is_empty() {
        return Err(invalid());
    }
    Ok((fields[0].clone(), fields[1].clone()))
}

#[cfg(test)]
#[path = "additional_writable_root_tests.rs"]
mod tests;
