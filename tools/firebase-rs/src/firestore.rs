//! Firebase Firestore REST API client
//!
//! Read-only access to Firestore documents for debugging and inspection.
//! Following AI-Foundation principle: read-only for safety.

use crate::client::FirebaseClient;
use crate::error::{FirebaseError, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;

/// Firestore client for document operations
pub struct FirestoreClient {
    client: Arc<FirebaseClient>,
}

/// Firestore document
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Document {
    /// Full document path
    pub name: String,
    /// Document fields
    pub fields: Option<HashMap<String, FirestoreValue>>,
    /// Create time
    #[serde(rename = "createTime")]
    pub create_time: Option<String>,
    /// Update time
    #[serde(rename = "updateTime")]
    pub update_time: Option<String>,
}

/// Firestore typed value
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FirestoreValue {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub null_value: Option<()>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub boolean_value: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub integer_value: Option<String>, // Firestore returns as string
    #[serde(skip_serializing_if = "Option::is_none")]
    pub double_value: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub string_value: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timestamp_value: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bytes_value: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reference_value: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub geo_point_value: Option<GeoPoint>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub array_value: Option<ArrayValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub map_value: Option<MapValue>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeoPoint {
    pub latitude: f64,
    pub longitude: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArrayValue {
    pub values: Option<Vec<FirestoreValue>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MapValue {
    pub fields: Option<HashMap<String, FirestoreValue>>,
}

/// List documents response
#[derive(Debug, Deserialize)]
struct ListDocumentsResponse {
    documents: Option<Vec<Document>>,
    #[serde(rename = "nextPageToken")]
    next_page_token: Option<String>,
}

/// Query request
#[derive(Debug, Serialize)]
struct RunQueryRequest {
    #[serde(rename = "structuredQuery")]
    structured_query: StructuredQuery,
}

#[derive(Debug, Serialize)]
struct StructuredQuery {
    from: Vec<CollectionSelector>,
    #[serde(rename = "where", skip_serializing_if = "Option::is_none")]
    where_clause: Option<Filter>,
    #[serde(skip_serializing_if = "Option::is_none")]
    limit: Option<i32>,
    #[serde(rename = "orderBy", skip_serializing_if = "Option::is_none")]
    order_by: Option<Vec<Order>>,
}

#[derive(Debug, Serialize)]
struct CollectionSelector {
    #[serde(rename = "collectionId")]
    collection_id: String,
    #[serde(rename = "allDescendants", skip_serializing_if = "Option::is_none")]
    all_descendants: Option<bool>,
}

#[derive(Debug, Serialize)]
struct Filter {
    #[serde(rename = "fieldFilter", skip_serializing_if = "Option::is_none")]
    field_filter: Option<FieldFilter>,
}

#[derive(Debug, Serialize)]
struct FieldFilter {
    field: FieldReference,
    op: String,
    value: FirestoreValue,
}

#[derive(Debug, Serialize)]
struct FieldReference {
    #[serde(rename = "fieldPath")]
    field_path: String,
}

#[derive(Debug, Serialize)]
struct Order {
    field: FieldReference,
    direction: String,
}

/// Query result
#[derive(Debug, Deserialize)]
struct QueryResult {
    document: Option<Document>,
    #[serde(rename = "readTime")]
    read_time: Option<String>,
}

impl FirestoreClient {
    /// Create new Firestore client
    pub fn new(client: Arc<FirebaseClient>) -> Self {
        Self { client }
    }

    /// Get a single document by path
    ///
    /// # Arguments
    /// * `path` - Document path (e.g., "users/abc123" or "workouts/session1")
    pub async fn get_document(&self, path: &str) -> Result<Document> {
        let url = self.client.api_url("firestore", path);
        self.client.get_json(&url).await
    }

    /// List documents in a collection
    ///
    /// # Arguments
    /// * `collection` - Collection path (e.g., "users" or "workouts")
    /// * `limit` - Maximum documents to return
    pub async fn list_documents(&self, collection: &str, limit: usize) -> Result<Vec<Document>> {
        let url = format!(
            "{}?pageSize={}",
            self.client.api_url("firestore", collection),
            limit
        );

        let response: ListDocumentsResponse = self.client.get_json(&url).await?;
        Ok(response.documents.unwrap_or_default())
    }

    /// Query documents with a filter
    ///
    /// # Arguments
    /// * `collection` - Collection ID
    /// * `field` - Field to filter on
    /// * `op` - Operator (EQUAL, NOT_EQUAL, LESS_THAN, etc.)
    /// * `value` - Value to compare against
    /// * `limit` - Maximum results
    pub async fn query(
        &self,
        collection: &str,
        field: &str,
        op: &str,
        value: &str,
        limit: usize,
    ) -> Result<Vec<Document>> {
        let url = format!(
            "https://firestore.googleapis.com/v1/projects/{}/databases/(default)/documents:runQuery",
            self.client.project_id()
        );

        let request = RunQueryRequest {
            structured_query: StructuredQuery {
                from: vec![CollectionSelector {
                    collection_id: collection.to_string(),
                    all_descendants: None,
                }],
                where_clause: Some(Filter {
                    field_filter: Some(FieldFilter {
                        field: FieldReference {
                            field_path: field.to_string(),
                        },
                        op: op.to_string(),
                        value: FirestoreValue {
                            string_value: Some(value.to_string()),
                            null_value: None,
                            boolean_value: None,
                            integer_value: None,
                            double_value: None,
                            timestamp_value: None,
                            bytes_value: None,
                            reference_value: None,
                            geo_point_value: None,
                            array_value: None,
                            map_value: None,
                        },
                    }),
                }),
                limit: Some(limit as i32),
                order_by: None,
            },
        };

        let response = self.client.post(&url, &request).await?;
        let results: Vec<QueryResult> = response.json().await?;

        Ok(results
            .into_iter()
            .filter_map(|r| r.document)
            .collect())
    }
}

impl FirestoreValue {
    /// Get value as a simple JSON-like representation
    pub fn to_simple(&self) -> Value {
        if self.null_value.is_some() {
            return Value::Null;
        }
        if let Some(b) = self.boolean_value {
            return Value::Bool(b);
        }
        if let Some(ref s) = self.integer_value {
            if let Ok(n) = s.parse::<i64>() {
                return Value::Number(n.into());
            }
        }
        if let Some(d) = self.double_value {
            return serde_json::Number::from_f64(d)
                .map(Value::Number)
                .unwrap_or(Value::Null);
        }
        if let Some(ref s) = self.string_value {
            return Value::String(s.clone());
        }
        if let Some(ref t) = self.timestamp_value {
            return Value::String(t.clone());
        }
        if let Some(ref r) = self.reference_value {
            return Value::String(format!("ref:{}", r));
        }
        if let Some(ref arr) = self.array_value {
            let values: Vec<_> = arr.values
                .as_ref()
                .map(|v| v.iter().map(|x| x.to_simple()).collect())
                .unwrap_or_default();
            return Value::Array(values);
        }
        if let Some(ref map) = self.map_value {
            let obj: serde_json::Map<String, Value> = map.fields
                .as_ref()
                .map(|f| f.iter().map(|(k, v)| (k.clone(), v.to_simple())).collect())
                .unwrap_or_default();
            return Value::Object(obj);
        }
        Value::Null
    }
}

impl Document {
    /// Get document ID from full path
    pub fn id(&self) -> &str {
        self.name.rsplit('/').next().unwrap_or(&self.name)
    }

    /// Convert fields to simple JSON
    pub fn to_json(&self) -> Value {
        let mut obj = serde_json::Map::new();
        obj.insert("_id".to_string(), Value::String(self.id().to_string()));

        if let Some(ref fields) = self.fields {
            for (key, value) in fields {
                obj.insert(key.clone(), value.to_simple());
            }
        }

        if let Some(ref create) = self.create_time {
            obj.insert("_createdAt".to_string(), Value::String(create.clone()));
        }
        if let Some(ref update) = self.update_time {
            obj.insert("_updatedAt".to_string(), Value::String(update.clone()));
        }

        Value::Object(obj)
    }
}

impl std::fmt::Display for Document {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let json = self.to_json();
        write!(f, "{}", serde_json::to_string_pretty(&json).unwrap_or_default())
    }
}
