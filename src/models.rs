use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Limits advertised in the published tool input schemas. rmcp deserializes
/// tool arguments with plain serde and never validates them against the
/// schema, so a client can send anything that parses; these are the values the
/// handlers enforce themselves. `runtime_limits_match_the_published_schema`
/// keeps the two from drifting apart.
pub const MAX_QUERY_CHARS: usize = 1000;
pub const MAX_FILTER_CHARS: usize = 500;
pub const MAX_ID_CHARS: usize = 255;
pub const MIN_ROWS: u8 = 1;
pub const MAX_ROWS: u8 = 20;

fn default_rows() -> u8 {
    10
}
fn default_search_field() -> SearchField {
    SearchField::AllFields
}

#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SearchField {
    AllFields,
    Title,
    Author,
    Subject,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CatalogSearchArgs {
    #[schemars(length(min = 1, max = 1000))]
    pub query: String,
    #[serde(default = "default_search_field")]
    pub search_field: SearchField,
    #[serde(default = "default_rows")]
    #[schemars(range(min = 1, max = 20))]
    pub rows: u8,
    #[serde(default)]
    pub filters: CatalogFilters,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CatalogFilters {
    #[schemars(length(max = 500))]
    pub access: Option<String>,
    #[schemars(length(max = 500))]
    pub format: Option<String>,
    #[schemars(length(max = 500))]
    pub library: Option<String>,
    #[schemars(length(max = 500))]
    pub genre: Option<String>,
    #[schemars(length(max = 500))]
    pub language: Option<String>,
    #[schemars(length(max = 500))]
    pub author: Option<String>,
    #[schemars(length(max = 500))]
    pub topic: Option<String>,
    #[schemars(length(max = 500))]
    pub region: Option<String>,
    #[schemars(length(max = 500))]
    pub call_number: Option<String>,
    #[schemars(length(max = 500))]
    pub era: Option<String>,
    #[schemars(length(max = 500))]
    pub organization_as_author: Option<String>,
}

/// Adding a filter here without adding its value below is a compile error, so
/// the request side (`pairs`) and the response side (`filter_name_for_facet`)
/// cannot disagree about which SearchWorks facet field a filter means. A wrong
/// field name is silently ignored upstream rather than rejected, so a drifting
/// entry would turn a filter into a no-op with no visible failure.
const FILTER_COUNT: usize = 11;
const FILTER_FIELDS: [(&str, &str); FILTER_COUNT] = [
    ("access", "access_facet"),
    ("format", "format_hsim"),
    ("library", "library"),
    ("genre", "genre_ssim"),
    ("language", "language"),
    ("author", "author_person_facet"),
    ("topic", "topic_facet"),
    ("region", "geographic_facet"),
    ("call_number", "callnum_facet_hsim"),
    ("era", "era_facet"),
    ("organization_as_author", "author_other_facet"),
];

/// The filter name that narrows a search by the given upstream facet field, if
/// this server exposes one. Used to key returned facets by the name a caller
/// passes back in `filters`.
pub fn filter_name_for_facet(upstream_field: &str) -> Option<&'static str> {
    FILTER_FIELDS
        .iter()
        .find(|(_, field)| *field == upstream_field)
        .map(|(name, _)| *name)
}

impl CatalogFilters {
    pub fn pairs(&self) -> impl Iterator<Item = (&'static str, &str, &'static str)> {
        let values: [Option<&str>; FILTER_COUNT] = [
            self.access.as_deref(),
            self.format.as_deref(),
            self.library.as_deref(),
            self.genre.as_deref(),
            self.language.as_deref(),
            self.author.as_deref(),
            self.topic.as_deref(),
            self.region.as_deref(),
            self.call_number.as_deref(),
            self.era.as_deref(),
            self.organization_as_author.as_deref(),
        ];
        FILTER_FIELDS
            .into_iter()
            .zip(values)
            .filter_map(|((friendly, field), value)| value.map(|v| (friendly, v, field)))
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ArticleSearchArgs {
    #[schemars(length(min = 1, max = 1000))]
    pub query: String,
    #[serde(default = "default_search_field")]
    pub search_field: SearchField,
    #[serde(default = "default_rows")]
    #[schemars(range(min = 1, max = 20))]
    pub rows: u8,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RecordArgs {
    #[schemars(length(min = 1, max = 255))]
    pub id: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct FacetValue {
    pub value: String,
    pub count: u64,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct Facet {
    pub label: String,
    pub values: Vec<FacetValue>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct CatalogResult {
    pub id: String,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pub_date: Option<String>,
    pub url: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub libraries: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub call_number: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct CatalogSearchOutput {
    pub query: String,
    pub search_field: String,
    pub filters: BTreeMap<String, String>,
    pub total: u64,
    pub results: Vec<CatalogResult>,
    pub facets: BTreeMap<String, Facet>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ArticleResult {
    pub id: String,
    pub title: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub authors: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub publication_date: Option<String>,
    #[serde(rename = "abstract", skip_serializing_if = "Option::is_none")]
    pub abstract_text: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub subjects: Vec<String>,
    pub url: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ArticleSearchOutput {
    pub query: String,
    pub search_field: String,
    pub total: u64,
    pub results: Vec<ArticleResult>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct RecordOutput {
    pub id: String,
    pub title: String,
    pub url: String,
    pub metadata: BTreeMap<String, Value>,
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    /// The upstream field names are SearchWorks facet field names, verified
    /// against the `facets[].name` values returned by `catalog.json`. A wrong
    /// name is silently ignored by Blacklight, so the filter would become a
    /// no-op rather than an error.
    #[test]
    fn maps_every_filter_to_its_upstream_facet_field() {
        let filters = CatalogFilters {
            access: Some("Online".into()),
            format: Some("Book".into()),
            library: Some("GREEN".into()),
            genre: Some("Bibliography".into()),
            language: Some("German".into()),
            author: Some("Ellington, Duke, 1899-1974".into()),
            topic: Some("Iron".into()),
            region: Some("Africa".into()),
            call_number: Some("T".into()),
            era: Some("1900-1999".into()),
            organization_as_author: Some("Stanford University".into()),
        };
        let mappings = filters
            .pairs()
            .map(|(friendly, _, field)| (friendly, field))
            .collect::<Vec<_>>();
        assert_eq!(
            mappings,
            [
                ("access", "access_facet"),
                ("format", "format_hsim"),
                ("library", "library"),
                ("genre", "genre_ssim"),
                ("language", "language"),
                ("author", "author_person_facet"),
                ("topic", "topic_facet"),
                ("region", "geographic_facet"),
                ("call_number", "callnum_facet_hsim"),
                ("era", "era_facet"),
                ("organization_as_author", "author_other_facet"),
            ]
        );
    }

    #[test]
    fn skips_filters_that_were_not_supplied() {
        let filters = CatalogFilters {
            library: Some("GREEN".into()),
            ..Default::default()
        };
        assert_eq!(
            filters.pairs().collect::<Vec<_>>(),
            [("library", "GREEN", "library")]
        );
    }

    #[test]
    fn runtime_limits_match_the_published_schema() {
        let catalog =
            serde_json::to_value(schemars::schema_for!(CatalogSearchArgs)).expect("schema");
        assert_eq!(
            catalog["properties"]["query"]["maxLength"],
            json!(MAX_QUERY_CHARS)
        );
        assert_eq!(catalog["properties"]["rows"]["minimum"], json!(MIN_ROWS));
        assert_eq!(catalog["properties"]["rows"]["maximum"], json!(MAX_ROWS));
        let filters = &catalog["$defs"]["CatalogFilters"]["properties"];
        for field in [
            "access",
            "format",
            "library",
            "genre",
            "language",
            "author",
            "topic",
            "region",
            "call_number",
            "era",
            "organization_as_author",
        ] {
            assert_eq!(
                filters[field]["maxLength"],
                json!(MAX_FILTER_CHARS),
                "filter {field}"
            );
        }

        let article =
            serde_json::to_value(schemars::schema_for!(ArticleSearchArgs)).expect("schema");
        assert_eq!(
            article["properties"]["query"]["maxLength"],
            json!(MAX_QUERY_CHARS)
        );
        assert_eq!(article["properties"]["rows"]["maximum"], json!(MAX_ROWS));

        let record = serde_json::to_value(schemars::schema_for!(RecordArgs)).expect("schema");
        assert_eq!(record["properties"]["id"]["maxLength"], json!(MAX_ID_CHARS));
    }
}
