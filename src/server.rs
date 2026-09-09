use std::collections::BTreeMap;

use rmcp::{
    ErrorData as McpError, ServerHandler,
    handler::server::wrapper::Parameters,
    model::{CallToolResult, ContentBlock},
    tool, tool_handler, tool_router,
};
use serde::Serialize;
use serde_json::{Value, json};

use crate::{
    client::SearchworksClient,
    eds,
    error::{ApiError, ErrorKind},
    models::{
        ArticleResult, ArticleSearchArgs, ArticleSearchOutput, CatalogFilters, CatalogResult,
        CatalogSearchArgs, CatalogSearchOutput, Facet, FacetValue, MAX_FILTER_CHARS, MAX_ID_CHARS,
        MAX_QUERY_CHARS, MAX_ROWS, MIN_ROWS, RecordArgs, RecordOutput, SearchField,
        filter_name_for_facet,
    },
};

#[derive(Clone)]
pub struct SearchworksMcp {
    client: SearchworksClient,
}

impl SearchworksMcp {
    pub fn new(client: SearchworksClient) -> Self {
        Self { client }
    }

    fn result<T: Serialize>(&self, text: String, value: &T) -> Result<CallToolResult, McpError> {
        let structured = serde_json::to_value(value)
            .map_err(|_| McpError::internal_error("Could not serialize tool response", None))?;
        let json_text = serde_json::to_string(&structured)
            .map_err(|_| McpError::internal_error("Could not serialize tool response", None))?;
        let mut result = CallToolResult::structured(structured);
        result.content = vec![ContentBlock::text(text), ContentBlock::text(json_text)];
        Ok(result)
    }

    /// `operation` names the failed call, e.g. "Catalog search". The rest of
    /// the message comes from the failure kind, so an agent can tell a bad id
    /// from an outage instead of retrying a request that can never succeed.
    fn api_error(
        &self,
        error: ApiError,
        operation: &'static str,
    ) -> Result<CallToolResult, McpError> {
        let kind = error.kind();
        if kind.retryable() {
            tracing::error!(error = %error, operation, "SearchWorks tool call failed");
        } else {
            tracing::warn!(error = %error, operation, "SearchWorks tool call failed");
        }
        let message = match kind {
            ErrorKind::NotFound => {
                format!(
                    "{operation} found no such record. Check that the id came from a search tool."
                )
            }
            ErrorKind::RateLimited => {
                format!("{operation} is rate limited upstream. Wait before retrying.")
            }
            ErrorKind::BadRequest => {
                format!("{operation} was rejected as an invalid request. Check the arguments.")
            }
            ErrorKind::Unavailable => format!("{operation} is temporarily unavailable."),
        };
        let structured = json!({ "error": message, "retryable": kind.retryable() });
        let mut result = CallToolResult::structured_error(structured);
        result.content = vec![ContentBlock::text(message)];
        Ok(result)
    }
}

#[tool_router]
impl SearchworksMcp {
    #[tool(
        name = "catalog_search_tool",
        description = "Search the Stanford library catalog for books, journals as cataloged publications, databases, media, archival collections, maps, and other library materials. Use article_search_tool instead for individual articles."
    )]
    async fn catalog_search(
        &self,
        Parameters(args): Parameters<CatalogSearchArgs>,
    ) -> Result<CallToolResult, McpError> {
        validate_query(&args.query)?;
        validate_filters(&args.filters)?;
        let rows = clamp_rows(args.rows);
        let search_field = catalog_field(args.search_field);
        let upstream_filters: BTreeMap<_, _> = args
            .filters
            .pairs()
            .map(|(_, value, field)| (field.to_string(), value.to_string()))
            .collect();
        let friendly_filters: BTreeMap<_, _> = args
            .filters
            .pairs()
            .map(|(key, value, _)| (key.to_string(), value.to_string()))
            .collect();
        let response = match self
            .client
            .catalog_search(&args.query, search_field, rows, &upstream_filters)
            .await
        {
            Ok(v) => v,
            Err(e) => return self.api_error(e, "Catalog search"),
        };
        let root = response
            .pointer("/response")
            .ok_or_else(|| McpError::internal_error("Invalid SearchWorks response", None))?;
        let results = root
            .get("docs")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .map(catalog_result)
            .collect::<Vec<_>>();
        let total = root
            .pointer("/pages/total_count")
            .and_then(Value::as_u64)
            .unwrap_or(results.len() as u64);
        let facets = parse_facets(root.get("facets"));
        let output = CatalogSearchOutput {
            query: args.query.clone(),
            search_field: friendly_field(args.search_field).into(),
            filters: friendly_filters,
            total,
            results,
            facets,
        };
        let text = catalog_text(&output);
        self.result(text, &output)
    }

    #[tool(
        name = "article_search_tool",
        description = "Search EDS for individual scholarly articles, journal articles, newspaper articles, and other academic publications. This returns metadata only; full text may require Stanford authentication."
    )]
    async fn article_search(
        &self,
        Parameters(args): Parameters<ArticleSearchArgs>,
    ) -> Result<CallToolResult, McpError> {
        validate_query(&args.query)?;
        let response = match self
            .client
            .article_search(
                &args.query,
                article_field(args.search_field),
                clamp_rows(args.rows),
            )
            .await
        {
            Ok(v) => v,
            Err(e) => return self.api_error(e, "Article search"),
        };
        let root = response
            .pointer("/response")
            .ok_or_else(|| McpError::internal_error("Invalid SearchWorks response", None))?;
        let results = root
            .get("docs")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .map(article_result)
            .collect::<Vec<_>>();
        let total = root
            .pointer("/pages/total_count")
            .and_then(Value::as_u64)
            .unwrap_or(results.len() as u64);
        let output = ArticleSearchOutput {
            query: args.query.clone(),
            search_field: friendly_field(args.search_field).into(),
            total,
            results,
        };
        let text = article_text(&output);
        self.result(text, &output)
    }

    #[tool(
        name = "get_catalog_record",
        description = "Retrieve detailed bibliographic metadata for one catalog result. Use an id returned by catalog_search_tool. This returns metadata, not full text."
    )]
    async fn catalog_record(
        &self,
        Parameters(args): Parameters<RecordArgs>,
    ) -> Result<CallToolResult, McpError> {
        validate_id(&args.id)?;
        let response = match self.client.catalog_record(&args.id).await {
            Ok(v) => v,
            Err(e) => {
                return self.api_error(e, "Catalog record retrieval");
            }
        };
        let doc = response
            .pointer("/response/document")
            .ok_or_else(|| McpError::internal_error("Invalid SearchWorks response", None))?;
        let title = first_string(doc, &["title_display", "title_full_display"])
            .unwrap_or_else(|| "Untitled".into());
        let metadata = catalog_metadata(doc);
        let output = RecordOutput {
            id: args.id.clone(),
            title,
            url: public_url("view", &args.id),
            metadata,
        };
        self.result(record_text(&output), &output)
    }

    #[tool(
        name = "get_article",
        description = "Retrieve detailed metadata and an available abstract for one article result. Use an id returned by article_search_tool. This does not return licensed article full text."
    )]
    async fn article_record(
        &self,
        Parameters(args): Parameters<RecordArgs>,
    ) -> Result<CallToolResult, McpError> {
        validate_id(&args.id)?;
        let response = match self.client.article_record(&args.id).await {
            Ok(v) => v,
            Err(e) => return self.api_error(e, "Article retrieval"),
        };
        let doc = response
            .pointer("/response/document")
            .ok_or_else(|| McpError::internal_error("Invalid SearchWorks response", None))?;
        let id = eds::id(doc).unwrap_or(args.id);
        let title = eds::title(doc).unwrap_or_else(|| "Untitled".into());
        let mut metadata = BTreeMap::new();
        put_values(&mut metadata, "authors", eds::authors(doc));
        put_opt(&mut metadata, "source", eds::source(doc));
        put_opt(
            &mut metadata,
            "publication_date",
            eds::publication_date(doc),
        );
        put_opt(
            &mut metadata,
            "publication_type",
            eds::publication_type(doc).or_else(|| eds::document_type(doc)),
        );
        put_opt(&mut metadata, "abstract", eds::abstract_text(doc));
        put_values(&mut metadata, "subjects", eds::subjects(doc));
        put_values(&mut metadata, "languages", eds::languages(doc));
        put_opt(&mut metadata, "publisher", eds::publisher(doc));
        put_opt(&mut metadata, "doi", eds::doi(doc));
        put_opt(&mut metadata, "volume", eds::volume(doc));
        put_opt(&mut metadata, "issue", eds::issue(doc));
        put_opt(&mut metadata, "start_page", eds::start_page(doc));
        let output = RecordOutput {
            id: id.clone(),
            title,
            url: public_url("articles", &id),
            metadata,
        };
        self.result(record_text(&output), &output)
    }
}

#[tool_handler(
    name = "searchworks",
    version = "0.1.0",
    instructions = "Choose the search tool matching the requested material. Use catalog_search_tool for books, whole journals, databases, media, archives, maps, and other catalog materials. Use article_search_tool for individual scholarly, journal, or newspaper articles. Use the corresponding get tool only when detailed metadata is needed. Cite the canonical SearchWorks URL returned by tools."
)]
impl ServerHandler for SearchworksMcp {}

fn validate_id(id: &str) -> Result<(), McpError> {
    if id.trim().is_empty() || id.chars().count() > MAX_ID_CHARS || id.chars().any(char::is_control)
    {
        return Err(McpError::invalid_params("id is invalid", None));
    }
    Ok(())
}

fn validate_query(query: &str) -> Result<(), McpError> {
    if query.trim().is_empty() {
        return Err(McpError::invalid_params("query must not be blank", None));
    }
    if query.chars().count() > MAX_QUERY_CHARS {
        return Err(McpError::invalid_params(
            format!("query must be at most {MAX_QUERY_CHARS} characters"),
            None,
        ));
    }
    Ok(())
}

fn validate_filters(filters: &CatalogFilters) -> Result<(), McpError> {
    for (name, value, _) in filters.pairs() {
        if value.chars().count() > MAX_FILTER_CHARS {
            return Err(McpError::invalid_params(
                format!("filter {name} must be at most {MAX_FILTER_CHARS} characters"),
                None,
            ));
        }
    }
    Ok(())
}

/// `rows` is a cap on how much to return, so a value outside the advertised
/// range is clamped rather than rejected: an out-of-range request still gets
/// usable results, and upstream is never asked for `per_page=0`, which it
/// answers with a 500.
fn clamp_rows(rows: u8) -> u8 {
    rows.clamp(MIN_ROWS, MAX_ROWS)
}

fn catalog_field(field: SearchField) -> &'static str {
    match field {
        SearchField::AllFields => "search",
        SearchField::Title => "search_title",
        SearchField::Author => "search_author",
        SearchField::Subject => "subject_terms",
    }
}
fn article_field(field: SearchField) -> &'static str {
    match field {
        SearchField::AllFields => "search",
        SearchField::Title => "title",
        SearchField::Author => "author",
        SearchField::Subject => "subject",
    }
}
fn friendly_field(field: SearchField) -> &'static str {
    match field {
        SearchField::AllFields => "all_fields",
        SearchField::Title => "title",
        SearchField::Author => "author",
        SearchField::Subject => "subject",
    }
}

fn catalog_result(doc: &Value) -> CatalogResult {
    let id = first_string(doc, &["id"]).unwrap_or_default();
    CatalogResult {
        id: id.clone(),
        title: first_string(doc, &["title_display", "title_full_display"])
            .unwrap_or_else(|| "Untitled".into()),
        author: first_string(
            doc,
            &["author_person_display", "author_person_full_display"],
        ),
        format: first_string(doc, &["format_main_ssim"]),
        pub_date: first_string(doc, &["pub_date"]),
        url: public_url("view", &id),
        libraries: strings(doc.get("holdings_library_code_ssim")),
        call_number: first_string(doc, &["lc_assigned_callnum_ssim"]),
    }
}

fn article_result(doc: &Value) -> ArticleResult {
    let id = first_string(doc, &["id"]).unwrap_or_default();
    let source_doc = doc.get("source");
    let mut abstract_text = first_string(doc, &["eds_abstract"]);
    if let Some(value) = abstract_text.as_mut()
        && value.chars().count() > 500
    {
        *value = value.chars().take(497).collect::<String>() + "...";
    }
    ArticleResult {
        id: id.clone(),
        title: first_string(doc, &["eds_title"]).unwrap_or_else(|| "Untitled".into()),
        authors: strings(doc.get("eds_authors")),
        source: first_string(doc, &["eds_composed_title", "eds_source_title"]),
        publication_date: first_string(doc, &["eds_publication_date"]),
        abstract_text,
        subjects: source_doc.map(eds::subjects).unwrap_or_default(),
        url: public_url("articles", &id),
    }
}

fn parse_facets(value: Option<&Value>) -> BTreeMap<String, Facet> {
    value
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|facet| {
            // Key each facet by the filter name that consumes it, taken from
            // the upstream field name. Deriving keys from display labels
            // instead produced names no filter accepted ("Organization (as
            // author)" became `organization__as_author`), and offered facets
            // this server cannot filter on at all -- and because the filter
            // object denies unknown fields, passing one back is a hard error
            // rather than a no-op.
            let key = filter_name_for_facet(facet.get("name")?.as_str()?)?;
            let label = facet.get("label")?.as_str()?.to_owned();
            let values = facet
                .get("items")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .take(5)
                .filter_map(|item| {
                    Some(FacetValue {
                        value: item.get("value")?.as_str()?.to_owned(),
                        count: item.get("hits")?.as_u64()?,
                    })
                })
                .collect();
            Some((key.to_owned(), Facet { label, values }))
        })
        .collect()
}

/// Each list is tried in order and the first non-empty one wins. The names
/// were checked against live `catalog.json` and `view/:id` responses; entries
/// that never appeared, and never rescued a document whose other names were
/// absent, are deliberately not listed.
fn catalog_metadata(doc: &Value) -> BTreeMap<String, Value> {
    let mut m = BTreeMap::new();
    for (key, fields) in [
        (
            "authors",
            &[
                "author_person_full_display",
                "author_person_display",
                "author_corp_display",
            ][..],
        ),
        ("formats", &["format_hsim", "format_main_ssim"][..]),
        ("publication_years", &["pub_year_tisim", "pub_date"][..]),
        ("languages", &["language"][..]),
        ("subjects", &["subject_all_search", "topic_facet"][..]),
        ("genres", &["genre_ssim"][..]),
        ("call_numbers", &["callnum_search"][..]),
        ("isbn", &["isbn_display"][..]),
        ("oclc", &["oclc"][..]),
    ] {
        let values = first_values(doc, fields);
        put_values(&mut m, key, values);
    }
    put_opt(
        &mut m,
        "publication",
        first_string(doc, &["imprint_display"]),
    );
    for (key, field) in [("summaries", "summary_struct"), ("contents", "toc_struct")] {
        let values = structured_text(doc.get(field));
        put_values(&mut m, key, values);
    }
    m
}

fn structured_text(value: Option<&Value>) -> Vec<String> {
    match value {
        Some(Value::Array(items)) => items
            .iter()
            .flat_map(|v| structured_text(Some(v)))
            .collect(),
        Some(Value::Object(map)) => {
            let label = map.get("label").and_then(Value::as_str);
            let nested = structured_text(map.get("fields").or_else(|| map.get("value")));
            if let Some(label) = label
                && !nested.is_empty()
            {
                return vec![format!("{label}: {}", nested.join("; "))];
            }
            nested
        }
        Some(Value::String(s)) if !s.is_empty() => vec![eds::sanitize_markup(s)],
        _ => vec![],
    }
}

fn first_values(doc: &Value, fields: &[&str]) -> Vec<String> {
    fields
        .iter()
        .map(|f| strings(doc.get(*f)))
        .find(|v| !v.is_empty())
        .unwrap_or_default()
}
fn first_string(doc: &Value, fields: &[&str]) -> Option<String> {
    first_values(doc, fields).into_iter().next()
}
fn strings(value: Option<&Value>) -> Vec<String> {
    match value {
        Some(Value::Array(v)) => v.iter().filter_map(scalar).collect(),
        Some(v) => scalar(v).into_iter().collect(),
        None => vec![],
    }
}
fn scalar(value: &Value) -> Option<String> {
    value
        .as_str()
        .map(eds::sanitize_markup)
        .or_else(|| value.as_i64().map(|n| n.to_string()))
}
fn put_values(map: &mut BTreeMap<String, Value>, key: &str, values: Vec<String>) {
    if !values.is_empty() {
        map.insert(key.into(), json!(values));
    }
}
fn put_opt(map: &mut BTreeMap<String, Value>, key: &str, value: Option<String>) {
    if let Some(value) = value.filter(|v| !v.is_empty()) {
        map.insert(key.into(), json!(value));
    }
}
fn public_url(kind: &str, id: &str) -> String {
    let encoded: String = url::form_urlencoded::byte_serialize(id.as_bytes()).collect();
    format!("https://searchworks.stanford.edu/{kind}/{encoded}")
}

fn catalog_text(output: &CatalogSearchOutput) -> String {
    if output.results.is_empty() {
        return format!("No results found for query: {}", output.query);
    }
    let rows = output
        .results
        .iter()
        .enumerate()
        .map(|(i, r)| format!("{}. {}\n   URL: {}", i + 1, r.title, r.url))
        .collect::<Vec<_>>()
        .join("\n\n");
    format!(
        "Found {} results (showing {}):\n\n{}",
        output.total,
        output.results.len(),
        rows
    )
}
fn article_text(output: &ArticleSearchOutput) -> String {
    if output.results.is_empty() {
        return format!("No articles found for query: {}", output.query);
    }
    let rows = output
        .results
        .iter()
        .enumerate()
        .map(|(i, r)| format!("{}. {}\n   URL: {}", i + 1, r.title, r.url))
        .collect::<Vec<_>>()
        .join("\n\n");
    format!(
        "Found {} articles (showing {}):\n\n{}",
        output.total,
        output.results.len(),
        rows
    )
}
fn record_text(output: &RecordOutput) -> String {
    let mut lines = vec![output.title.clone(), format!("URL: {}", output.url)];
    for (key, value) in &output.metadata {
        lines.push(format!("{}: {}", key.replace('_', " "), value));
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    /// Shaped after a real `catalog.json` doc. Search docs carry holdings in
    /// `holdings_library_code_ssim` / `lc_assigned_callnum_ssim`; the bare
    /// `library` and `callnum_display` names exist only on other endpoints, so
    /// reading them here silently yielded null for every result.
    fn search_doc() -> Value {
        json!({
            "id": "994811",
            "title_display": "Rust [poems]",
            "author_person_display": ["Hilberry, Conrad."],
            "format_main_ssim": ["Book"],
            "pub_date": "1974",
            "holdings_library_code_ssim": ["SCIENCE", "GREEN"],
            "lc_assigned_callnum_ssim": ["PS3558.I384.R8"]
        })
    }

    /// Shaped after the real `facets` array, including the two entries whose
    /// labels used to produce unusable keys.
    fn facets() -> Value {
        json!([
            { "name": "access_facet", "label": "Access",
              "items": [{ "value": "Online", "hits": 3417 }, { "value": "At the Library", "hits": 12 }] },
            { "name": "author_other_facet", "label": "Organization (as author)",
              "items": [{ "value": "Stanford University", "hits": 7 }] },
            { "name": "library", "label": "Library",
              "items": [{ "value": "GREEN", "hits": 325 }] },
            { "name": "pub_year_tisim", "label": "Date",
              "items": [{ "value": "2020", "hits": 5 }] },
            { "name": "collection", "label": "Collection",
              "items": [{ "value": "Some collection", "hits": 2 }] }
        ])
    }

    #[test]
    fn keys_facets_by_the_filter_name_that_consumes_them() {
        let parsed = parse_facets(Some(&facets()));
        assert_eq!(
            parsed.keys().collect::<Vec<_>>(),
            ["access", "library", "organization_as_author"]
        );
        assert_eq!(parsed["access"].label, "Access");
        assert_eq!(parsed["access"].values[0].value, "Online");
        assert_eq!(parsed["access"].values[0].count, 3417);
        // "Organization (as author)" used to key as `organization__as_author`.
        assert_eq!(
            parsed["organization_as_author"].label,
            "Organization (as author)"
        );
    }

    /// The point of the facet block is to tell the caller what to filter on
    /// next, so anything this server cannot filter on is left out.
    #[test]
    fn omits_facets_with_no_matching_filter() {
        let parsed = parse_facets(Some(&facets()));
        assert!(!parsed.contains_key("date"));
        assert!(!parsed.contains_key("collection"));
        assert!(!parsed.contains_key("pub_year_tisim"));
    }

    #[test]
    fn every_returned_facet_key_is_an_accepted_filter_name() {
        let parsed = parse_facets(Some(&facets()));
        for key in parsed.keys() {
            let json = json!({ key: "x" });
            serde_json::from_value::<CatalogFilters>(json)
                .unwrap_or_else(|e| panic!("facet key {key} is not a valid filter: {e}"));
        }
    }

    #[test]
    fn caps_facet_values() {
        let items = (0..9)
            .map(|n| json!({ "value": n.to_string(), "hits": 1 }))
            .collect::<Vec<_>>();
        let parsed = parse_facets(Some(&json!([
            { "name": "access_facet", "label": "Access", "items": items }
        ])));
        assert_eq!(parsed["access"].values.len(), 5);
    }

    #[test]
    fn handles_missing_or_malformed_facets() {
        assert!(parse_facets(None).is_empty());
        assert!(parse_facets(Some(&json!([]))).is_empty());
        assert!(parse_facets(Some(&json!("nonsense"))).is_empty());
        assert!(parse_facets(Some(&json!([{ "label": "No name field" }]))).is_empty());
    }

    #[test]
    fn clamps_rows_into_the_advertised_range() {
        assert_eq!(clamp_rows(0), MIN_ROWS);
        assert_eq!(clamp_rows(1), 1);
        assert_eq!(clamp_rows(10), 10);
        assert_eq!(clamp_rows(20), 20);
        assert_eq!(clamp_rows(200), MAX_ROWS);
        assert_eq!(clamp_rows(u8::MAX), MAX_ROWS);
    }

    #[test]
    fn rejects_blank_and_overlong_queries() {
        assert!(validate_query("rust").is_ok());
        assert!(validate_query("   ").is_err());
        assert!(validate_query("").is_err());
        assert!(validate_query(&"a".repeat(MAX_QUERY_CHARS)).is_ok());
        assert!(validate_query(&"a".repeat(MAX_QUERY_CHARS + 1)).is_err());
    }

    /// The schema counts characters, so the runtime check has to as well; a
    /// byte-length check would reject queries well inside the advertised limit
    /// once they contain non-ASCII text.
    #[test]
    fn measures_limits_in_characters_not_bytes() {
        let multibyte = "\u{4e16}".repeat(MAX_QUERY_CHARS);
        assert!(
            multibyte.len() > MAX_QUERY_CHARS,
            "should exceed byte limit"
        );
        assert!(validate_query(&multibyte).is_ok());
        assert!(validate_id(&"\u{4e16}".repeat(MAX_ID_CHARS)).is_ok());
        assert!(validate_id(&"\u{4e16}".repeat(MAX_ID_CHARS + 1)).is_err());
    }

    #[test]
    fn rejects_overlong_filter_values() {
        let ok = CatalogFilters {
            topic: Some("a".repeat(MAX_FILTER_CHARS)),
            ..Default::default()
        };
        assert!(validate_filters(&ok).is_ok());
        let too_long = CatalogFilters {
            topic: Some("a".repeat(MAX_FILTER_CHARS + 1)),
            ..Default::default()
        };
        assert!(validate_filters(&too_long).is_err());
    }

    #[test]
    fn rejects_invalid_ids() {
        assert!(validate_id("994811").is_ok());
        assert!(validate_id("  ").is_err());
        assert!(validate_id("bad\u{0007}id").is_err());
    }

    #[test]
    fn projects_holdings_from_search_doc_fields() {
        let result = catalog_result(&search_doc());
        assert_eq!(result.libraries, ["SCIENCE", "GREEN"]);
        assert_eq!(result.call_number.as_deref(), Some("PS3558.I384.R8"));
        assert_eq!(result.title, "Rust [poems]");
        assert_eq!(result.format.as_deref(), Some("Book"));
        assert_eq!(result.pub_date.as_deref(), Some("1974"));
        assert_eq!(result.url, "https://searchworks.stanford.edu/view/994811");
    }

    #[test]
    fn omits_holdings_when_the_doc_has_none() {
        let result = catalog_result(&json!({ "id": "1", "title_display": "T" }));
        assert!(result.libraries.is_empty());
        assert_eq!(result.call_number, None);
    }
}
