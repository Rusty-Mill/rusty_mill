//! Sovereign zero-dependency HTTP JSON remote cloud search provider for `rusty_search`.
//!
//! **Not yet implemented.** [`CloudSearchBackend`] is a placeholder: it does not
//! perform any network calls against a remote search API. Every
//! [`SearchBackend`] method returns [`SearchError::Backend`] describing the
//! missing implementation instead of silently reporting fabricated success, so
//! callers cannot mistake this backend for a working one.

#![deny(missing_docs)]

use async_trait::async_trait;
use rusty_search_core::{
    Document, Schema, SearchBackend, SearchError, SearchRequest, SearchResults,
};

/// Sovereign Remote Cloud Search Provider client.
///
/// Not yet implemented: every [`SearchBackend`] method returns
/// [`SearchError::Backend`] rather than talking to `endpoint`.
pub struct CloudSearchBackend {
    endpoint: String,
}

impl CloudSearchBackend {
    /// Creates a new CloudSearchBackend with remote API endpoint URI.
    pub fn new(endpoint: &str) -> Self {
        Self {
            endpoint: String::from(endpoint),
        }
    }

    /// Builds the "not implemented" error returned by every backend method.
    fn not_implemented(&self, operation: &str) -> SearchError {
        SearchError::backend_msg(format!(
            "CloudSearchBackend::{operation} is not yet implemented (endpoint: {})",
            self.endpoint
        ))
    }
}

#[async_trait]
impl SearchBackend for CloudSearchBackend {
    async fn create_index(&self, _index_name: &str, _schema: Schema) -> Result<(), SearchError> {
        Err(self.not_implemented("create_index"))
    }

    async fn delete_index(&self, _index_name: &str) -> Result<(), SearchError> {
        Err(self.not_implemented("delete_index"))
    }

    async fn index_exists(&self, _index_name: &str) -> Result<bool, SearchError> {
        Err(self.not_implemented("index_exists"))
    }

    async fn index(&self, _index_name: &str, _doc: Document) -> Result<(), SearchError> {
        Err(self.not_implemented("index"))
    }

    async fn index_batch(
        &self,
        _index_name: &str,
        _docs: Vec<Document>,
    ) -> Result<(), SearchError> {
        Err(self.not_implemented("index_batch"))
    }

    async fn delete(&self, _index_name: &str, _doc_id: &str) -> Result<(), SearchError> {
        Err(self.not_implemented("delete"))
    }

    async fn commit(&self, _index_name: &str) -> Result<(), SearchError> {
        Err(self.not_implemented("commit"))
    }

    async fn search(
        &self,
        _index_name: &str,
        _request: SearchRequest,
    ) -> Result<SearchResults, SearchError> {
        Err(self.not_implemented("search"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusty_search_core::Query;

    #[test]
    fn backend_initialization() {
        let backend = CloudSearchBackend::new("https://search.rusty-mill.org");
        assert_eq!(backend.endpoint, "https://search.rusty-mill.org");
    }

    #[tokio::test]
    async fn every_method_reports_not_implemented_instead_of_fake_success() {
        let backend = CloudSearchBackend::new("https://search.rusty-mill.org");

        assert!(backend.index("docs", Document::new()).await.is_err());
        assert!(backend
            .search("docs", SearchRequest::new(Query::match_all()))
            .await
            .is_err());
        assert!(backend.index_exists("docs").await.is_err());
    }
}
