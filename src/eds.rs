use std::sync::LazyLock;

use regex::Regex;
use scraper::Html;
use serde_json::Value;

pub fn id(document: &Value) -> Option<String> {
    let header = document.get("Header")?;
    Some(format!(
        "{}__{}",
        header.get("DbId")?.as_str()?,
        header.get("An")?.as_str()?
    ))
}

pub fn title(document: &Value) -> Option<String> {
    bib_entity(document)
        .and_then(|e| e.get("Titles"))?
        .as_array()?
        .iter()
        .find(|x| x.get("Type").and_then(Value::as_str) == Some("main"))
        .and_then(|x| x.get("TitleFull"))
        .and_then(Value::as_str)
        .map(sanitize_markup)
        .or_else(|| item(document, Some("Title"), None, None))
}

pub fn authors(document: &Value) -> Vec<String> {
    let mut values = Vec::new();
    deep_strings(
        document.pointer("/RecordInfo/BibRecord/BibRelationships"),
        "NameFull",
        &mut values,
    );
    let mut unique = Vec::new();
    for value in values {
        if !unique.contains(&value) {
            unique.push(value);
        }
    }
    unique
}

pub fn source(document: &Value) -> Option<String> {
    document
        .pointer("/RecordInfo/BibRecord/BibRelationships/IsPartOfRelationships/0/BibEntity/Titles")
        .and_then(Value::as_array)
        .and_then(|titles| {
            titles
                .iter()
                .find(|x| x.get("Type").and_then(Value::as_str) == Some("main"))
        })
        .and_then(|x| x.get("TitleFull"))
        .and_then(Value::as_str)
        .map(sanitize_markup)
        .or_else(|| item(document, Some("TitleSource"), None, None))
}

pub fn publication_date(document: &Value) -> Option<String> {
    let date = document
        .pointer("/RecordInfo/BibRecord/BibRelationships/IsPartOfRelationships/0/BibEntity/Dates")
        .and_then(Value::as_array)
        .and_then(|dates| {
            dates
                .iter()
                .find(|d| d.get("Type").and_then(Value::as_str) == Some("published"))
        });
    match date {
        Some(d) if d.get("Y").is_some() && d.get("M").is_some() && d.get("D").is_some() => {
            Some(format!(
                "{}-{}-{}",
                scalar(d.get("Y")?)?,
                scalar(d.get("M")?)?,
                scalar(d.get("D")?)?
            ))
        }
        _ => item(document, Some("DatePub"), None, None),
    }
}

pub fn publication_type(document: &Value) -> Option<String> {
    document
        .pointer("/Header/PubType")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .or_else(|| item(document, Some("TypePub"), None, None))
}

pub fn document_type(document: &Value) -> Option<String> {
    item(document, Some("TypeDocument"), None, None)
}
pub fn abstract_text(document: &Value) -> Option<String> {
    item(document, Some("Abstract"), None, None)
}
pub fn publisher(document: &Value) -> Option<String> {
    item(document, Some("Publisher"), None, None)
}
pub fn languages(document: &Value) -> Vec<String> {
    item(document, Some("Language"), None, None)
        .map(|v| vec![v])
        .unwrap_or_else(|| {
            bib_entity(document)
                .and_then(|e| e.get("Languages"))
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|x| x.get("Text").and_then(Value::as_str).map(str::to_owned))
                .collect()
        })
}
pub fn doi(document: &Value) -> Option<String> {
    item(document, Some("DOI"), None, None).or_else(|| {
        bib_entity(document)?
            .get("Identifiers")?
            .as_array()?
            .iter()
            .find(|x| x.get("Type").and_then(Value::as_str) == Some("doi"))?
            .get("Value")?
            .as_str()
            .map(str::to_owned)
    })
}
pub fn volume(document: &Value) -> Option<String> {
    numbering(document, "volume")
}
pub fn issue(document: &Value) -> Option<String> {
    numbering(document, "issue")
}
pub fn start_page(document: &Value) -> Option<String> {
    let mut values = Vec::new();
    deep_strings(bib_entity(document), "StartPage", &mut values);
    values.into_iter().next()
}
pub fn subjects(document: &Value) -> Vec<String> {
    let raw = item(document, Some("Subject"), Some("Subject Terms"), Some("Su"))
        .or_else(|| {
            item(
                document,
                Some("Subject"),
                Some("Subject Indexing"),
                Some("Su"),
            )
        })
        .or_else(|| {
            item(
                document,
                Some("Subject"),
                Some("Subject Category"),
                Some("Su"),
            )
        });
    raw.map(|s| {
        s.split(" -- ")
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .collect()
    })
    .unwrap_or_default()
}

fn bib_entity(document: &Value) -> Option<&Value> {
    document.pointer("/RecordInfo/BibRecord/BibEntity")
}
fn numbering(document: &Value, kind: &str) -> Option<String> {
    document
        .pointer("/RecordInfo/BibRecord/BibRelationships/IsPartOfRelationships/0/Numbering")?
        .as_array()?
        .iter()
        .find(|x| x.get("Type").and_then(Value::as_str) == Some(kind))?
        .get("Value")
        .and_then(scalar)
}
fn scalar(value: &Value) -> Option<String> {
    value
        .as_str()
        .map(str::to_owned)
        .or_else(|| value.as_i64().map(|n| n.to_string()))
}
fn item(
    document: &Value,
    name: Option<&str>,
    label: Option<&str>,
    group: Option<&str>,
) -> Option<String> {
    let found = document.get("Items")?.as_array()?.iter().find(|entry| {
        name.is_none_or(|v| entry.get("Name").and_then(Value::as_str) == Some(v))
            && label.is_none_or(|v| entry.get("Label").and_then(Value::as_str) == Some(v))
            && group.is_none_or(|v| entry.get("Group").and_then(Value::as_str) == Some(v))
    })?;
    found.get("Data")?.as_str().map(sanitize_markup)
}
fn deep_strings(value: Option<&Value>, key: &str, out: &mut Vec<String>) {
    match value {
        Some(Value::Object(map)) => {
            for (k, v) in map {
                if k == key {
                    if let Some(s) = v.as_str() {
                        out.push(sanitize_markup(s));
                    }
                } else {
                    deep_strings(Some(v), key, out);
                }
            }
        }
        Some(Value::Array(values)) => {
            for value in values {
                deep_strings(Some(value), key, out);
            }
        }
        _ => {}
    }
}
pub fn sanitize_markup(input: &str) -> String {
    static CONTROL_CHARACTERS: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"[\x00-\x08\x0b\x0c\x0e-\x1f\x7f]").expect("valid regex"));
    static WHITESPACE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s+").expect("valid regex"));
    static SPACE_BEFORE_PUNCTUATION: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"\s+([.,;:!?])").expect("valid regex"));
    let decoded = html_escape::decode_html_entities(input);
    let fragment = Html::parse_fragment(&decoded);
    let text = fragment.root_element().text().collect::<Vec<_>>().join(" ");
    let safe = CONTROL_CHARACTERS.replace_all(text.trim(), "");
    let normalized = WHITESPACE.replace_all(&safe, " ");
    SPACE_BEFORE_PUNCTUATION
        .replace_all(&normalized, "$1")
        .into_owned()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn document() -> Value {
        json!({
            "Header": { "DbId": "edsspo", "An": "springer.123", "PubType": "Academic Journal" },
            "Items": [
                { "Name": "Abstract", "Data": "A &amp; B <b>abstract</b>." },
                { "Name": "DOI", "Data": "10.123/example" },
                { "Name": "Subject", "Label": "Subject Terms", "Group": "Su", "Data": "Libraries -- Catalogs" }
            ],
            "RecordInfo": { "BibRecord": {
                "BibEntity": {
                    "Titles": [{ "Type": "main", "TitleFull": "An <i>article</i>" }],
                    "Languages": [{ "Text": "English" }],
                    "PhysicalDescription": { "Pagination": { "StartPage": "3" } }
                },
                "BibRelationships": {
                    "HasContributorRelationships": [{ "PersonEntity": { "Name": { "NameFull": "First Author" } } }],
                    "IsPartOfRelationships": [{
                        "BibEntity": {
                            "Titles": [{ "Type": "main", "TitleFull": "A Journal" }],
                            "Dates": [{ "Type": "published", "Y": "2026", "M": "08", "D": "20" }]
                        },
                        "Numbering": [{ "Type": "volume", "Value": "4" }, { "Type": "issue", "Value": "2" }]
                    }]
                }
            }}
        })
    }

    #[test]
    fn extracts_curated_article_metadata() {
        let doc = document();
        assert_eq!(id(&doc).as_deref(), Some("edsspo__springer.123"));
        assert_eq!(title(&doc).as_deref(), Some("An article"));
        assert_eq!(authors(&doc), ["First Author"]);
        assert_eq!(source(&doc).as_deref(), Some("A Journal"));
        assert_eq!(publication_date(&doc).as_deref(), Some("2026-08-20"));
        assert_eq!(abstract_text(&doc).as_deref(), Some("A & B abstract."));
        assert_eq!(subjects(&doc), ["Libraries", "Catalogs"]);
        assert_eq!(languages(&doc), ["English"]);
        assert_eq!(volume(&doc).as_deref(), Some("4"));
        assert_eq!(issue(&doc).as_deref(), Some("2"));
        assert_eq!(start_page(&doc).as_deref(), Some("3"));
    }

    #[test]
    fn strips_markup_and_unsafe_control_characters() {
        assert_eq!(
            sanitize_markup("hello<script>bad()</script>\u{0007}<b>world</b>"),
            "hello bad() world"
        );
    }
}
