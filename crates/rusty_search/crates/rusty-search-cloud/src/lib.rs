//! Sovereign zero-dependency HTTP JSON remote cloud search provider for `rusty_search`.

#![deny(missing_docs)]

use async_trait::async_trait;
use rusty_search_core::{
    Document, Schema, SearchBackend, SearchError, SearchRequest, SearchResults,
};

/// Sovereign Remote Cloud Search Provider client.
pub struct CloudSearchBackend {
    #[allow(dead_code)] // not yet wired into any SearchBackend method below
    endpoint: String,
}

impl CloudSearchBackend {
    /// Creates a new CloudSearchBackend with remote API endpoint URI.
    pub fn new(endpoint: &str) -> Self {
        Self {
            endpoint: String::from(endpoint),
        }
    }
}

#[async_trait]
impl SearchBackend for CloudSearchBackend {
    async fn create_index(&self, _index_name: &str, _schema: Schema) -> Result<(), SearchError> {
        Ok(())
    }

    async fn delete_index(&self, _index_name: &str) -> Result<(), SearchError> {
        Ok(())
    }

    async fn index_exists(&self, _index_name: &str) -> Result<bool, SearchError> {
        Ok(true)
    }

    async fn index(&self, _index_name: &str, _doc: Document) -> Result<(), SearchError> {
        Ok(())
    }

    async fn index_batch(
        &self,
        _index_name: &str,
        _docs: Vec<Document>,
    ) -> Result<(), SearchError> {
        Ok(())
    }

    async fn delete(&self, _index_name: &str, _doc_id: &str) -> Result<(), SearchError> {
        Ok(())
    }

    async fn commit(&self, _index_name: &str) -> Result<(), SearchError> {
        Ok(())
    }

    async fn search(
        &self,
        _index_name: &str,
        _request: SearchRequest,
    ) -> Result<SearchResults, SearchError> {
        Ok(SearchResults::empty())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backend_initialization() {
        let backend = CloudSearchBackend::new("https://search.rusty-mill.org");
        assert_eq!(backend.endpoint, "https://search.rusty-mill.org");
    }
}
