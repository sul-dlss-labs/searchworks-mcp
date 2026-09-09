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
/// Every source database labels its subject items differently (`Subjects`,
/// `Descriptors`, `Geographic Terms`, `Categories`, ...), so match on the `Su`
/// group rather than an allow-list of labels, and read every matching item
/// rather than only the first.
pub fn subjects(document: &Value) -> Vec<String> {
    let mut unique = Vec::new();
    for data in subject_data(document) {
        for line in markup_lines(data) {
            for term in line.split(" -- ").map(str::trim) {
                if !term.is_empty() && !unique.iter().any(|existing| existing == term) {
                    unique.push(term.to_owned());
                }
            }
        }
    }
    unique
}

fn subject_data(document: &Value) -> impl Iterator<Item = &str> {
    document
        .get("Items")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|entry| entry.get("Group").and_then(Value::as_str) == Some("Su"))
        .filter_map(|entry| entry.get("Data").and_then(Value::as_str))
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
    strip_markup(&html_escape::decode_html_entities(input))
}

/// EDS packs several values into one `Data` string separated by `<br />`. The
/// breaks have to be split before markup is stripped: stripping turns them
/// into ordinary spaces, after which the values are indistinguishable from one
/// run-on string.
fn markup_lines(input: &str) -> Vec<String> {
    static BREAK_TAG: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?i)<br\s*/?>").expect("valid regex"));
    let decoded = html_escape::decode_html_entities(input);
    BREAK_TAG
        .split(&decoded)
        .map(strip_markup)
        .filter(|line| !line.is_empty())
        .collect()
}

fn strip_markup(decoded: &str) -> String {
    static CONTROL_CHARACTERS: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"[\x00-\x08\x0b\x0c\x0e-\x1f\x7f]").expect("valid regex"));
    static WHITESPACE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s+").expect("valid regex"));
    static SPACE_BEFORE_PUNCTUATION: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"\s+([.,;:!?])").expect("valid regex"));
    let fragment = Html::parse_fragment(decoded);
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

    /// Real `eric__EJ1497355` shape: two subject items, neither carrying one of
    /// the labels the old allow-list looked for, each packing several terms
    /// into one `Data` string separated by `<br />`.
    #[test]
    fn collects_subjects_from_every_labelled_group_and_splits_breaks() {
        let descriptors = "&lt;searchLink fieldCode=&quot;DE&quot; term=&quot;%22Video+Games%22&quot;&gt;Video Games&lt;/searchLink&gt;&lt;br /&gt;&lt;searchLink fieldCode=&quot;DE&quot;&gt;History Instruction&lt;/searchLink&gt;&lt;br /&gt;Medieval History";
        let geographic = "&lt;searchLink&gt;Spain&lt;/searchLink&gt;&lt;br /&gt;United Kingdom";
        let doc = json!({
            "Items": [
                { "Name": "Subject", "Label": "Descriptors", "Group": "Su", "Data": descriptors },
                { "Name": "Subject", "Label": "Geographic Terms", "Group": "Su", "Data": geographic },
                { "Name": "Abstract", "Label": "Abstract", "Group": "Ab", "Data": "Not a subject." }
            ]
        });
        assert_eq!(
            subjects(&doc),
            [
                "Video Games",
                "History Instruction",
                "Medieval History",
                "Spain",
                "United Kingdom"
            ]
        );
    }

    /// Real `nlebk__805805` shape: LCSH headings keep their `--` subdivisions,
    /// and BISAC categories are a separate item in the same `Su` group.
    #[test]
    fn keeps_subdivided_headings_and_includes_bisac_categories() {
        let doc = json!({
            "Items": [
                { "Name": "Subject", "Label": "Subjects", "Group": "Su",
                  "Data": "Video games--History&lt;br /&gt;Video games--Social aspects" },
                { "Name": "SubjectBISAC", "Label": "Categories", "Group": "Su",
                  "Data": "GAMES &amp;amp; ACTIVITIES / Board Games" }
            ]
        });
        assert_eq!(
            subjects(&doc),
            [
                "Video games--History",
                "Video games--Social aspects",
                "GAMES & ACTIVITIES / Board Games"
            ]
        );
    }

    #[test]
    fn deduplicates_subjects_repeated_across_items() {
        let doc = json!({
            "Items": [
                { "Name": "Subject", "Group": "Su", "Data": "Gene therapy&lt;br /&gt;Genetics" },
                { "Name": "SubjectBISAC", "Group": "Su", "Data": "Genetics" }
            ]
        });
        assert_eq!(subjects(&doc), ["Gene therapy", "Genetics"]);
    }

    #[test]
    fn returns_no_subjects_when_the_record_has_none() {
        assert!(subjects(&json!({ "Items": [] })).is_empty());
        assert!(subjects(&json!({})).is_empty());
    }

    #[test]
    fn strips_markup_and_unsafe_control_characters() {
        assert_eq!(
            sanitize_markup("hello<script>bad()</script>\u{0007}<b>world</b>"),
            "hello bad() world"
        );
    }
}
