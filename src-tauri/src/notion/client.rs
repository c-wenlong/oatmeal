//! The Notion HTTP API, in the narrow slice this app needs.
//!
//! Deliberately small: list the databases the integration can see, read one
//! database's property names, create a page, replace a page's children. That is
//! the whole surface, and every part of it is exercised by the export flow —
//! there is no speculative wrapper here.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// The API version this code was written against.
///
/// Notion requires the header and treats it as a contract: sending no version,
/// or a newer one than the code understands, changes response shapes silently.
pub const API_VERSION: &str = "2022-06-28";

#[derive(Debug, thiserror::Error)]
pub enum NotionError {
    #[error("could not reach Notion: {0}")]
    Unreachable(String),
    #[error("Notion rejected the request ({status}): {message}")]
    Api { status: u16, message: String },
    #[error("unexpected response from Notion: {0}")]
    Malformed(String),
    #[error("no Notion token is stored")]
    NoToken,
}

/// A database the integration has been shared with.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Database {
    pub id: String,
    pub title: String,
    /// The name of the title column, which the user chose and we cannot guess.
    pub title_property: String,
    /// Every property name, so the export only sends ones that exist.
    pub properties: Vec<String>,
}

pub struct Notion {
    http: reqwest::Client,
    base_url: String,
    token: String,
}

impl Notion {
    pub fn new(token: impl Into<String>) -> Self {
        Self::with_base_url("https://api.notion.com", token)
    }

    /// Points at a different host. Only used by tests, which stand up a local
    /// server rather than talking to Notion.
    pub fn with_base_url(base_url: impl Into<String>, token: impl Into<String>) -> Self {
        Self {
            http: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(60))
                .build()
                .unwrap_or_default(),
            base_url: base_url.into().trim_end_matches('/').to_string(),
            token: token.into(),
        }
    }

    fn request(&self, method: reqwest::Method, path: &str) -> reqwest::RequestBuilder {
        self.http
            .request(method, format!("{}{path}", self.base_url))
            .bearer_auth(&self.token)
            .header("Notion-Version", API_VERSION)
    }

    async fn send(&self, request: reqwest::RequestBuilder) -> Result<Value, NotionError> {
        let response = request
            .send()
            .await
            .map_err(|e| NotionError::Unreachable(e.to_string()))?;

        let status = response.status();
        let body = response
            .text()
            .await
            .map_err(|e| NotionError::Unreachable(e.to_string()))?;

        if !status.is_success() {
            // Notion puts a human-readable reason in `message`; surfacing the
            // raw body instead would show the user a wall of JSON.
            let message = serde_json::from_str::<Value>(&body)
                .ok()
                .and_then(|v| v["message"].as_str().map(str::to_string))
                .unwrap_or_else(|| body.chars().take(200).collect());
            return Err(NotionError::Api {
                status: status.as_u16(),
                message,
            });
        }

        serde_json::from_str(&body).map_err(|e| NotionError::Malformed(e.to_string()))
    }

    /// Databases the integration can see.
    ///
    /// Notion only returns what the user has explicitly shared with the
    /// integration, which is why an empty list is a normal, expected state and
    /// the UI has to explain it rather than look broken.
    pub async fn databases(&self) -> Result<Vec<Database>, NotionError> {
        let body = json!({
            "filter": { "property": "object", "value": "database" },
            "page_size": 100
        });
        let value = self
            .send(
                self.request(reqwest::Method::POST, "/v1/search")
                    .json(&body),
            )
            .await?;

        let results = value["results"]
            .as_array()
            .ok_or_else(|| NotionError::Malformed("no results array".into()))?;

        Ok(results.iter().filter_map(parse_database).collect())
    }

    /// Creates a page in a database and returns its id.
    pub async fn create_page(
        &self,
        database_id: &str,
        properties: Value,
        children: Vec<Value>,
    ) -> Result<String, NotionError> {
        let body = json!({
            "parent": { "database_id": database_id },
            "properties": properties,
            "children": children,
        });
        let value = self
            .send(self.request(reqwest::Method::POST, "/v1/pages").json(&body))
            .await?;

        value["id"]
            .as_str()
            .map(str::to_string)
            .ok_or_else(|| NotionError::Malformed("page id missing from response".into()))
    }

    pub async fn update_properties(
        &self,
        page_id: &str,
        properties: Value,
    ) -> Result<(), NotionError> {
        self.send(
            self.request(reqwest::Method::PATCH, &format!("/v1/pages/{page_id}"))
                .json(&json!({ "properties": properties })),
        )
        .await?;
        Ok(())
    }

    pub async fn append_children(
        &self,
        page_id: &str,
        children: Vec<Value>,
    ) -> Result<(), NotionError> {
        self.send(
            self.request(
                reqwest::Method::PATCH,
                &format!("/v1/blocks/{page_id}/children"),
            )
            .json(&json!({ "children": children })),
        )
        .await?;
        Ok(())
    }

    /// Ids of a page's existing children.
    pub async fn children(&self, page_id: &str) -> Result<Vec<String>, NotionError> {
        let value = self
            .send(self.request(
                reqwest::Method::GET,
                &format!("/v1/blocks/{page_id}/children?page_size=100"),
            ))
            .await?;
        Ok(value["results"]
            .as_array()
            .map(|results| {
                results
                    .iter()
                    .filter_map(|block| block["id"].as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default())
    }

    /// Deletes a block. Notion calls this "archiving".
    pub async fn delete_block(&self, block_id: &str) -> Result<(), NotionError> {
        self.send(self.request(reqwest::Method::DELETE, &format!("/v1/blocks/{block_id}")))
            .await?;
        Ok(())
    }
}

/// Reads a database out of a search result.
///
/// Split out and public to the crate so the shape parsing — which is where a
/// Notion response change would bite — is testable without a network.
pub fn parse_database(value: &Value) -> Option<Database> {
    let id = value["id"].as_str()?.to_string();

    // A database's title is an array of rich-text runs; an untitled one has an
    // empty array rather than a missing field.
    let title = value["title"]
        .as_array()
        .map(|runs| {
            runs.iter()
                .filter_map(|run| run["plain_text"].as_str())
                .collect::<String>()
        })
        .filter(|t| !t.trim().is_empty())
        .unwrap_or_else(|| "Untitled database".to_string());

    let properties = value["properties"].as_object()?;
    let names: Vec<String> = properties.keys().cloned().collect();

    // Exactly one property has type `title`, and its name is whatever the user
    // called it. Guessing "Name" works until someone renames the column, and
    // then every export fails with an unhelpful Notion error.
    let title_property = properties
        .iter()
        .find(|(_, definition)| definition["type"] == "title")
        .map(|(name, _)| name.clone())?;

    Some(Database {
        id,
        title,
        title_property,
        properties: names,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn database_json() -> Value {
        json!({
            "id": "db-1",
            "title": [{ "plain_text": "Meeting " }, { "plain_text": "notes" }],
            "properties": {
                "Meeting": { "type": "title" },
                "Date": { "type": "date" },
                "Duration": { "type": "number" }
            }
        })
    }

    #[test]
    fn a_database_is_parsed_with_its_title_column() {
        let db = parse_database(&database_json()).unwrap();
        assert_eq!(db.id, "db-1");
        assert_eq!(db.title, "Meeting notes");
        // Not "Name": the user renamed it, and guessing would fail every export.
        assert_eq!(db.title_property, "Meeting");
        assert!(db.properties.contains(&"Date".to_string()));
    }

    #[test]
    fn a_database_with_no_title_column_is_skipped() {
        // Not a database we can create a page in; offering it would produce an
        // export that fails at the last step.
        let value =
            json!({ "id": "db", "title": [], "properties": { "Date": { "type": "date" } } });
        assert!(parse_database(&value).is_none());
    }

    #[test]
    fn an_untitled_database_still_gets_a_label() {
        let value = json!({
            "id": "db",
            "title": [],
            "properties": { "Name": { "type": "title" } }
        });
        assert_eq!(parse_database(&value).unwrap().title, "Untitled database");
    }

    #[test]
    fn a_malformed_entry_is_skipped_rather_than_failing_the_list() {
        // One odd row in a workspace should not make the picker empty.
        assert!(parse_database(&json!({ "id": "db" })).is_none());
        assert!(parse_database(&json!({})).is_none());
    }

    #[test]
    fn the_base_url_tolerates_a_trailing_slash() {
        let notion = Notion::with_base_url("http://localhost:9/", "t");
        assert_eq!(notion.base_url, "http://localhost:9");
    }

    #[tokio::test]
    async fn an_unreachable_host_is_reported_as_such() {
        let notion = Notion::with_base_url("http://127.0.0.1:1", "token");
        let err = notion.databases().await.unwrap_err();
        assert!(matches!(err, NotionError::Unreachable(_)), "{err:?}");
    }
}
