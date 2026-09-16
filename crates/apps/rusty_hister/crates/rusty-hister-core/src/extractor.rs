use crate::document::{Document, Metadata};
use crate::error::HisterError;

/// Which independent roles an extractor plays — capability inventory §4.1's
/// `Capabilities{Enrich, Extract, Preview bool}`. Independent booleans, not
/// an enum: an extractor can enrich without ever producing the primary
/// extracted text (e.g. Hister's `EmbeddedVideo` and `JSON-LD` extractors
/// are enrich-only, capability inventory §4.6).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Capabilities {
    /// Runs during the enrich phase — never stops the chain on fallback,
    /// only on abort (capability inventory §4.2).
    pub enrich: bool,
    /// Can produce the document's primary extracted content.
    pub extract: bool,
    /// Can produce a rendered preview.
    pub preview: bool,
}

/// Per-extractor configuration: whether it's enabled, plus a free-form
/// options bag merged from the extractor's own defaults and user config
/// (capability inventory §4.6 — e.g. `ytdlp`'s `extra_domains`). Enabled by
/// default, matching Hister's own default chain (every extractor runs
/// unless explicitly disabled).
#[derive(Debug, Clone)]
pub struct ExtractorConfig {
    /// Whether this extractor participates in the chain at all.
    pub enabled: bool,
    /// Extractor-specific options, validated by the extractor itself.
    pub options: Metadata,
}

impl Default for ExtractorConfig {
    fn default() -> Self {
        ExtractorConfig {
            enabled: true,
            options: Metadata::new(),
        }
    }
}

/// A rendered preview: the plain text, HTML, and metadata capability
/// inventory §2.3's `get_preview` (and the HTTP `/api/preview` route, row
/// 1.22) return for a document.
#[derive(Debug, Clone, Default)]
pub struct PreviewResponse {
    /// Rendered (or raw) HTML for display.
    pub html: Option<String>,
    /// Plain-text rendition, if available separately from `html`.
    pub text: Option<String>,
    /// Metadata harvested while rendering the preview (e.g. Readability's
    /// OpenGraph/JSON-LD-derived fields, capability inventory §4.4).
    pub metadata: Metadata,
}

/// The three-way outcome of [`Extractor::extract`] — capability inventory
/// §4.1's `ExtractorSuccess` / `ExtractorFallback` / `ExtractorAbort` tri-state.
/// A closed Rust enum reproduces the same three states Hister's Go code
/// hides behind opaque-type factory functions (`Extracted()`,
/// `ExtractFallback(err)`, `AbortExtraction(err)`) without needing that
/// indirection: matching on this enum can't observe or construct a fourth
/// state.
#[derive(Debug)]
pub enum ExtractOutcome {
    /// Extraction succeeded; stop the chain and use this document.
    Extracted(Document),
    /// This extractor doesn't apply after all (or hit a recoverable
    /// problem) — try the next extractor in chain order.
    Fallback(HisterError),
    /// A fatal problem: stop the whole chain, don't try further extractors.
    Abort(HisterError),
}

/// The three-way outcome of [`Extractor::preview`], mirroring
/// [`ExtractOutcome`] for the preview chain (capability inventory §4.2's
/// separate preview-chain semantics).
#[derive(Debug)]
pub enum PreviewOutcome {
    /// Preview rendering succeeded.
    Previewed(PreviewResponse),
    /// Try the next extractor in chain order.
    Fallback(HisterError),
    /// Stop the whole preview chain.
    Abort(HisterError),
}

/// The contract every extractor implements — a Rust port of capability
/// inventory §4.1's `Extractor` interface. Synchronous: every extractor
/// operates on an already-fetched [`Document`] (HTML/text the crawler
/// already retrieved), so there's no I/O here to make async worth it
/// (matching the async-only-for-real-concurrency default; Hister's own
/// optional `ContextExtractor`/`ContextPreviewer` variants exist for
/// extractors that need cancellation, not extractors that need async I/O,
/// and aren't part of this bootstrap-phase contract).
pub trait Extractor: Send + Sync {
    /// The extractor's registry name (case-insensitive elsewhere in the
    /// chain — capability inventory §4.2's lower-cased config keys).
    fn name(&self) -> &str;
    /// A human-readable description (surfaced by `/api/extractors`, row
    /// 1.23).
    fn description(&self) -> &str;
    /// Which roles this extractor plays.
    fn capabilities(&self) -> Capabilities;
    /// Whether this extractor applies to `document`.
    fn matches(&self, document: &Document) -> bool;
    /// Attempts to extract `document`'s primary content.
    fn extract(&self, document: &Document) -> ExtractOutcome;
    /// Attempts to render a preview of `document`.
    fn preview(&self, document: &Document) -> PreviewOutcome;
    /// The extractor's current configuration.
    fn config(&self) -> &ExtractorConfig;
    /// Replaces this extractor's configuration, validating it first.
    fn set_config(&mut self, config: ExtractorConfig) -> Result<(), HisterError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A minimal extractor used only to exercise the trait's contract
    /// shape and the tri-state outcomes — not a real port of any Hister
    /// extractor.
    struct AlwaysMatchExtractor {
        config: ExtractorConfig,
    }

    impl Extractor for AlwaysMatchExtractor {
        fn name(&self) -> &str {
            "always-match"
        }

        fn description(&self) -> &str {
            "Test-only extractor that matches every document."
        }

        fn capabilities(&self) -> Capabilities {
            Capabilities {
                enrich: false,
                extract: true,
                preview: true,
            }
        }

        fn matches(&self, _document: &Document) -> bool {
            true
        }

        fn extract(&self, document: &Document) -> ExtractOutcome {
            if document.url.is_empty() {
                return ExtractOutcome::Abort(HisterError::Extraction("empty url".to_string()));
            }
            let mut extracted = document.clone();
            extracted.text = Some("extracted".to_string());
            ExtractOutcome::Extracted(extracted)
        }

        fn preview(&self, _document: &Document) -> PreviewOutcome {
            PreviewOutcome::Previewed(PreviewResponse {
                html: Some("<p>preview</p>".to_string()),
                text: Some("preview".to_string()),
                metadata: Metadata::new(),
            })
        }

        fn config(&self) -> &ExtractorConfig {
            &self.config
        }

        fn set_config(&mut self, config: ExtractorConfig) -> Result<(), HisterError> {
            if config
                .options
                .contains_key("unknown-key-should-be-rejected")
            {
                return Err(HisterError::InvalidConfig(
                    "unknown-key-should-be-rejected".to_string(),
                ));
            }
            self.config = config;
            Ok(())
        }
    }

    fn extractor() -> AlwaysMatchExtractor {
        AlwaysMatchExtractor {
            config: ExtractorConfig::default(),
        }
    }

    #[test]
    fn capabilities_default_to_all_false() {
        assert_eq!(
            Capabilities::default(),
            Capabilities {
                enrich: false,
                extract: false,
                preview: false,
            }
        );
    }

    #[test]
    fn extractor_config_defaults_to_enabled_with_no_options() {
        let config = ExtractorConfig::default();
        assert!(config.enabled);
        assert!(config.options.is_empty());
    }

    #[test]
    fn extract_returns_extracted_on_success() {
        let ext = extractor();
        let doc = Document::new("https://example.com");
        match ext.extract(&doc) {
            ExtractOutcome::Extracted(result) => {
                assert_eq!(result.text.as_deref(), Some("extracted"));
            }
            other => panic!("expected Extracted, got {other:?}"),
        }
    }

    #[test]
    fn extract_returns_abort_on_fatal_problem() {
        let ext = extractor();
        let doc = Document::new("");
        match ext.extract(&doc) {
            ExtractOutcome::Abort(HisterError::Extraction(msg)) => {
                assert_eq!(msg, "empty url");
            }
            other => panic!("expected Abort(Extraction), got {other:?}"),
        }
    }

    #[test]
    fn preview_returns_rendered_response() {
        let ext = extractor();
        let doc = Document::new("https://example.com");
        match ext.preview(&doc) {
            PreviewOutcome::Previewed(response) => {
                assert_eq!(response.html.as_deref(), Some("<p>preview</p>"));
            }
            other => panic!("expected Previewed, got {other:?}"),
        }
    }

    #[test]
    fn set_config_rejects_unknown_option_key() {
        let mut ext = extractor();
        let mut options = Metadata::new();
        options.insert(
            "unknown-key-should-be-rejected".to_string(),
            rusty_json::Value::Bool(true),
        );
        let result = ext.set_config(ExtractorConfig {
            enabled: true,
            options,
        });
        assert!(matches!(result, Err(HisterError::InvalidConfig(_))));
    }

    #[test]
    fn set_config_accepts_valid_config() {
        let mut ext = extractor();
        let result = ext.set_config(ExtractorConfig {
            enabled: false,
            options: Metadata::new(),
        });
        assert!(result.is_ok());
        assert!(!ext.config().enabled);
    }

    fn assert_object_safe(_: &dyn Extractor) {}

    #[test]
    fn extractor_trait_is_object_safe() {
        let ext = extractor();
        assert_object_safe(&ext);
    }
}
