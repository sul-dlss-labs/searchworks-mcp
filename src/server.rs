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
        ArticleResult, ArticleSearchArgs, ArticleSearchOutput, CatalogResult, CatalogSearchArgs,
        CatalogSearchOutput, Facet, FacetValue, RecordArgs, RecordOutput, SearchField,
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
        if args.query.trim().is_empty() {
            return Err(McpError::invalid_params("query must not be blank", None));
        }
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
            .catalog_search(&args.query, search_field, args.rows, &upstream_filters)
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
        if args.query.trim().is_empty() {
            return Err(McpError::invalid_params("query must not be blank", None));
        }
        let response = match self
            .client
            .article_search(&args.query, article_field(args.search_field), args.rows)
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
    if id.trim().is_empty() || id.len() > 255 || id.chars().any(char::is_control) {
        return Err(McpError::invalid_params("id is invalid", None));
    }
    Ok(())
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
        format: first_string(doc, &["format", "format_main_ssim"]),
        pub_date: first_string(doc, &["pub_date", "pub_year_tisim"]),
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
            let label = facet.get("label")?.as_str()?.to_owned();
            let key = label
                .to_lowercase()
                .chars()
                .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
                .collect::<String>()
                .trim_matches('_')
                .to_owned();
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
            Some((key, Facet { label, values }))
        })
        .collect()
}

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
        (
            "formats",
            &["format_hsim", "format", "format_main_ssim"][..],
        ),
        ("publication_years", &["pub_year_tisim", "pub_date"][..]),
        ("languages", &["language"][..]),
        ("subjects", &["subject_all_search", "topic_facet"][..]),
        ("genres", &["genre_ssim"][..]),
        ("call_numbers", &["callnum_display", "callnum_search"][..]),
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
