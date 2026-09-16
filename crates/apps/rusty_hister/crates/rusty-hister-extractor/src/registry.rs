use std::collections::HashMap;

use rusty_hister_core::{
    Document, ExtractOutcome, Extractor, ExtractorConfig, HisterError, PreviewOutcome,
};

/// The ordered chain-of-responsibility every document runs through — a
/// Rust port of capability inventory §4.2's `Registry`. Extractor lookup
/// by name is case-insensitive throughout (matching viper's lower-cased
/// YAML config keys, which is also why user-supplied config in
/// [`Registry::apply_configs`] is keyed case-insensitively).
///
/// Not internally synchronized: Hister's Go `Registry` guards its list
/// with a mutex because it's shared across concurrent HTTP handlers: this
/// type makes no such assumption (wrap it in a `Mutex`/`RwLock` at the call
/// site, e.g. inside `rusty-hister-server`, if concurrent access is
/// needed) — no I/O or shared state to synchronize here, so building that
/// in now would be exactly the speculative generality this cluster's own
/// `AGENTS.md` warns against.
#[derive(Default)]
pub struct Registry {
    extractors: Vec<Box<dyn Extractor>>,
}

impl Registry {
    /// An empty registry.
    pub fn new() -> Self {
        Registry::default()
    }

    /// Registers `extractor` at the end of the chain. Errors if an
    /// extractor with the same name (case-insensitive) is already
    /// registered.
    pub fn register(&mut self, extractor: Box<dyn Extractor>) -> Result<(), HisterError> {
        self.reject_duplicate(extractor.name())?;
        self.extractors.push(extractor);
        Ok(())
    }

    /// Registers `extractor` immediately before the extractor named
    /// `before_name` (case-insensitive) — an insertion point for
    /// extension points that must run ahead of an existing extractor.
    /// Errors if `before_name` doesn't exist, or if `extractor`'s own
    /// name is already registered.
    pub fn register_before(
        &mut self,
        before_name: &str,
        extractor: Box<dyn Extractor>,
    ) -> Result<(), HisterError> {
        self.reject_duplicate(extractor.name())?;
        let index = self.find_index(before_name).ok_or_else(|| {
            HisterError::InvalidConfig(format!(
                "no extractor named `{before_name}` to insert before"
            ))
        })?;
        self.extractors.insert(index, extractor);
        Ok(())
    }

    /// Merges `configs` (extractor name, case-insensitive → the config to
    /// apply) into the matching registered extractors, validating each via
    /// [`Extractor::set_config`]. Unknown names in `configs` are ignored —
    /// mirroring `Init`'s "config for an extractor that isn't registered"
    /// case, which Hister treats as a no-op rather than an error, since
    /// config files commonly list every known extractor whether or not a
    /// given build registered it. Parsing a config *file* (YAML in
    /// Hister's case) into this map is a separate, not-yet-decided
    /// concern — this method only merges an already-parsed map.
    pub fn apply_configs(
        &mut self,
        configs: HashMap<String, ExtractorConfig>,
    ) -> Result<(), HisterError> {
        let lower: HashMap<String, ExtractorConfig> = configs
            .into_iter()
            .map(|(name, config)| (name.to_ascii_lowercase(), config))
            .collect();
        for extractor in &mut self.extractors {
            if let Some(config) = lower.get(&extractor.name().to_ascii_lowercase()) {
                extractor.set_config(config.clone())?;
            }
        }
        Ok(())
    }

    /// Every registered extractor, in chain order.
    pub fn list(&self) -> impl Iterator<Item = &dyn Extractor> {
        self.extractors.iter().map(Box::as_ref)
    }

    /// Every registered, enabled extractor, in chain order.
    pub fn list_enabled(&self) -> impl Iterator<Item = &dyn Extractor> {
        self.enabled().map(|(_, e)| e)
    }

    /// Names of enabled extractors that match `document` for extraction or
    /// enrichment.
    pub fn list_matching(&self, document: &Document) -> Vec<&str> {
        self.enabled()
            .filter(|(_, e)| {
                (e.capabilities().extract || e.capabilities().enrich) && e.matches(document)
            })
            .map(|(_, e)| e.name())
            .collect()
    }

    /// Names of enabled, preview-capable extractors that match `document`.
    pub fn list_matching_preview(&self, document: &Document) -> Vec<&str> {
        self.enabled()
            .filter(|(_, e)| e.capabilities().preview && e.matches(document))
            .map(|(_, e)| e.name())
            .collect()
    }

    /// Runs the two-phase extraction chain (capability inventory §4.2):
    /// every matching enabled enricher runs in chain order first — a
    /// `Fallback` from an enricher is skipped over (enrichers never stop
    /// the chain that way), only `Abort` halts everything — carrying its
    /// enrichment forward into the next stage; then matching enabled
    /// content extractors run in chain order until one succeeds or
    /// aborts.
    pub fn extract(&self, document: &Document) -> ExtractOutcome {
        let mut current = document.clone();

        for (_, extractor) in self.enabled() {
            if extractor.capabilities().enrich && extractor.matches(&current) {
                match extractor.extract(&current) {
                    ExtractOutcome::Extracted(enriched) => current = enriched,
                    ExtractOutcome::Fallback(_) => {}
                    ExtractOutcome::Abort(err) => return ExtractOutcome::Abort(err),
                }
            }
        }

        let mut last_fallback: Option<HisterError> = None;
        for (_, extractor) in self.enabled() {
            if extractor.capabilities().extract && extractor.matches(&current) {
                match extractor.extract(&current) {
                    ExtractOutcome::Extracted(result) => return ExtractOutcome::Extracted(result),
                    ExtractOutcome::Fallback(err) => last_fallback = Some(err),
                    ExtractOutcome::Abort(err) => return ExtractOutcome::Abort(err),
                }
            }
        }

        ExtractOutcome::Fallback(last_fallback.unwrap_or_else(|| {
            HisterError::Extraction("no extractor matched this document".to_string())
        }))
    }

    /// Runs the preview chain (capability inventory §4.2), separate from
    /// [`Registry::extract`]'s chain. `starting_point`, when given, names
    /// (case-insensitively) the extractor to start from — every extractor
    /// after it in chain order still runs as fallback, so naming one
    /// doesn't disable the rest of the chain, it just skips ahead to it.
    /// A `starting_point` that's unregistered, disabled, not
    /// preview-capable, or doesn't match `document` is a hard error
    /// (`Abort`), never silently ignored.
    pub fn preview(&self, document: &Document, starting_point: Option<&str>) -> PreviewOutcome {
        let start_index = match starting_point {
            None => 0,
            Some(name) => match self.validated_starting_point(name, document) {
                Ok(index) => index,
                Err(err) => return PreviewOutcome::Abort(err),
            },
        };

        let mut last_fallback: Option<HisterError> = None;
        for extractor in &self.extractors[start_index..] {
            let config = extractor.config();
            if config.enabled && extractor.capabilities().preview && extractor.matches(document) {
                match extractor.preview(document) {
                    PreviewOutcome::Previewed(response) => {
                        return PreviewOutcome::Previewed(response)
                    }
                    PreviewOutcome::Fallback(err) => last_fallback = Some(err),
                    PreviewOutcome::Abort(err) => return PreviewOutcome::Abort(err),
                }
            }
        }

        PreviewOutcome::Fallback(last_fallback.unwrap_or_else(|| {
            HisterError::Extraction("no extractor matched this document".to_string())
        }))
    }

    fn validated_starting_point(
        &self,
        name: &str,
        document: &Document,
    ) -> Result<usize, HisterError> {
        let index = self
            .find_index(name)
            .ok_or_else(|| HisterError::InvalidConfig(format!("no extractor named `{name}`")))?;
        let extractor = &self.extractors[index];
        if !extractor.config().enabled {
            return Err(HisterError::InvalidConfig(format!(
                "extractor `{name}` is disabled"
            )));
        }
        if !extractor.capabilities().preview {
            return Err(HisterError::InvalidConfig(format!(
                "extractor `{name}` does not support preview"
            )));
        }
        if !extractor.matches(document) {
            return Err(HisterError::InvalidConfig(format!(
                "extractor `{name}` does not match this document"
            )));
        }
        Ok(index)
    }

    fn enabled(&self) -> impl Iterator<Item = (usize, &dyn Extractor)> {
        self.extractors
            .iter()
            .enumerate()
            .filter(|(_, e)| e.config().enabled)
            .map(|(i, e)| (i, e.as_ref()))
    }

    fn find_index(&self, name: &str) -> Option<usize> {
        self.extractors
            .iter()
            .position(|e| e.name().eq_ignore_ascii_case(name))
    }

    fn reject_duplicate(&self, name: &str) -> Result<(), HisterError> {
        if self.find_index(name).is_some() {
            return Err(HisterError::InvalidConfig(format!(
                "extractor `{name}` is already registered"
            )));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusty_hister_core::{Capabilities, PreviewResponse};

    /// A test-only extractor whose behavior (match/extract/preview
    /// outcomes) is fully scripted, so registry tests can exercise every
    /// branch of the chain logic without depending on any real Hister
    /// extractor's parsing behavior.
    struct ScriptedExtractor {
        name: &'static str,
        capabilities: Capabilities,
        config: ExtractorConfig,
        matches: bool,
        extract_outcome: Box<dyn Fn(&Document) -> ExtractOutcome + Send + Sync>,
        preview_outcome: Box<dyn Fn(&Document) -> PreviewOutcome + Send + Sync>,
    }

    impl ScriptedExtractor {
        fn new(name: &'static str, capabilities: Capabilities) -> Self {
            ScriptedExtractor {
                name,
                capabilities,
                config: ExtractorConfig::default(),
                matches: true,
                extract_outcome: Box::new(|_document| {
                    ExtractOutcome::Fallback(HisterError::Extraction("unscripted".to_string()))
                }),
                preview_outcome: Box::new(|_document| {
                    PreviewOutcome::Fallback(HisterError::Extraction("unscripted".to_string()))
                }),
            }
        }

        fn disabled(mut self) -> Self {
            self.config.enabled = false;
            self
        }

        fn non_matching(mut self) -> Self {
            self.matches = false;
            self
        }

        fn extracting(
            mut self,
            outcome: impl Fn(&Document) -> ExtractOutcome + Send + Sync + 'static,
        ) -> Self {
            self.extract_outcome = Box::new(outcome);
            self
        }

        fn previewing(
            mut self,
            outcome: impl Fn(&Document) -> PreviewOutcome + Send + Sync + 'static,
        ) -> Self {
            self.preview_outcome = Box::new(outcome);
            self
        }

        fn boxed(self) -> Box<dyn Extractor> {
            Box::new(self)
        }
    }

    impl Extractor for ScriptedExtractor {
        fn name(&self) -> &str {
            self.name
        }

        fn description(&self) -> &str {
            "scripted test extractor"
        }

        fn capabilities(&self) -> Capabilities {
            self.capabilities
        }

        fn matches(&self, _document: &Document) -> bool {
            self.matches
        }

        fn extract(&self, document: &Document) -> ExtractOutcome {
            (self.extract_outcome)(document)
        }

        fn preview(&self, document: &Document) -> PreviewOutcome {
            (self.preview_outcome)(document)
        }

        fn config(&self) -> &ExtractorConfig {
            &self.config
        }

        fn set_config(&mut self, config: ExtractorConfig) -> Result<(), HisterError> {
            self.config = config;
            Ok(())
        }
    }

    fn extract_caps() -> Capabilities {
        Capabilities {
            enrich: false,
            extract: true,
            preview: false,
        }
    }

    fn enrich_caps() -> Capabilities {
        Capabilities {
            enrich: true,
            extract: false,
            preview: false,
        }
    }

    fn preview_caps() -> Capabilities {
        Capabilities {
            enrich: false,
            extract: false,
            preview: true,
        }
    }

    fn doc() -> Document {
        Document::new("https://example.com")
    }

    #[test]
    fn register_rejects_case_insensitive_duplicate_names() {
        let mut registry = Registry::new();
        registry
            .register(ScriptedExtractor::new("Reddit", extract_caps()).boxed())
            .unwrap();
        let err = registry
            .register(ScriptedExtractor::new("reddit", extract_caps()).boxed())
            .unwrap_err();
        assert!(matches!(err, HisterError::InvalidConfig(_)));
    }

    #[test]
    fn register_before_inserts_at_the_right_position() {
        let mut registry = Registry::new();
        registry
            .register(ScriptedExtractor::new("basic", extract_caps()).boxed())
            .unwrap();
        registry
            .register_before(
                "basic",
                ScriptedExtractor::new("readability", extract_caps()).boxed(),
            )
            .unwrap();
        let names: Vec<&str> = registry.list().map(Extractor::name).collect();
        assert_eq!(names, vec!["readability", "basic"]);
    }

    #[test]
    fn register_before_errors_on_unknown_target() {
        let mut registry = Registry::new();
        let err = registry
            .register_before(
                "missing",
                ScriptedExtractor::new("x", extract_caps()).boxed(),
            )
            .unwrap_err();
        assert!(matches!(err, HisterError::InvalidConfig(_)));
    }

    #[test]
    fn extract_stops_at_first_successful_content_extractor() {
        let mut registry = Registry::new();
        registry
            .register(
                ScriptedExtractor::new("first", extract_caps())
                    .extracting(|_| {
                        ExtractOutcome::Fallback(HisterError::Extraction(
                            "not this one".to_string(),
                        ))
                    })
                    .boxed(),
            )
            .unwrap();
        registry
            .register(
                ScriptedExtractor::new("second", extract_caps())
                    .extracting(|_| {
                        ExtractOutcome::Extracted(Document::new("https://example.com/second"))
                    })
                    .boxed(),
            )
            .unwrap();
        registry
            .register(
                ScriptedExtractor::new("third", extract_caps())
                    .extracting(|_| panic!("must not run: chain should have stopped at `second`"))
                    .boxed(),
            )
            .unwrap();

        match registry.extract(&doc()) {
            ExtractOutcome::Extracted(result) => {
                assert_eq!(result.url, "https://example.com/second");
            }
            other => panic!("expected Extracted, got {other:?}"),
        }
    }

    #[test]
    fn extract_aborts_immediately_on_abort() {
        let mut registry = Registry::new();
        registry
            .register(
                ScriptedExtractor::new("aborts", extract_caps())
                    .extracting(|_| {
                        ExtractOutcome::Abort(HisterError::Extraction("fatal".to_string()))
                    })
                    .boxed(),
            )
            .unwrap();
        registry
            .register(
                ScriptedExtractor::new("never-runs", extract_caps())
                    .extracting(|_| panic!("must not run after an abort"))
                    .boxed(),
            )
            .unwrap();

        match registry.extract(&doc()) {
            ExtractOutcome::Abort(HisterError::Extraction(msg)) => assert_eq!(msg, "fatal"),
            other => panic!("expected Abort, got {other:?}"),
        }
    }

    #[test]
    fn extract_skips_non_matching_and_disabled_extractors() {
        let mut registry = Registry::new();
        registry
            .register(
                ScriptedExtractor::new("non-matching", extract_caps())
                    .non_matching()
                    .extracting(|_| panic!("must not run: doesn't match"))
                    .boxed(),
            )
            .unwrap();
        registry
            .register(
                ScriptedExtractor::new("disabled", extract_caps())
                    .disabled()
                    .extracting(|_| panic!("must not run: disabled"))
                    .boxed(),
            )
            .unwrap();
        registry
            .register(
                ScriptedExtractor::new("real", extract_caps())
                    .extracting(|_| {
                        ExtractOutcome::Extracted(Document::new("https://example.com/real"))
                    })
                    .boxed(),
            )
            .unwrap();

        match registry.extract(&doc()) {
            ExtractOutcome::Extracted(result) => assert_eq!(result.url, "https://example.com/real"),
            other => panic!("expected Extracted, got {other:?}"),
        }
    }

    #[test]
    fn extract_runs_enrichment_before_extraction_and_carries_it_forward() {
        use std::sync::{Arc, Mutex};

        let seen_label = Arc::new(Mutex::new(None));
        let seen_label_in_extractor = Arc::clone(&seen_label);

        let mut registry = Registry::new();
        registry
            .register(
                ScriptedExtractor::new("enricher", enrich_caps())
                    .extracting(|document| {
                        let mut enriched = document.clone();
                        enriched.label = Some("enriched".to_string());
                        ExtractOutcome::Extracted(enriched)
                    })
                    .boxed(),
            )
            .unwrap();
        registry
            .register(
                ScriptedExtractor::new("extractor", extract_caps())
                    .extracting(move |document| {
                        *seen_label_in_extractor.lock().unwrap() = document.label.clone();
                        ExtractOutcome::Extracted(document.clone())
                    })
                    .boxed(),
            )
            .unwrap();

        match registry.extract(&doc()) {
            ExtractOutcome::Extracted(_) => {
                assert_eq!(
                    *seen_label.lock().unwrap(),
                    Some("enriched".to_string()),
                    "the content extractor should have seen the enricher's output, not the original document"
                );
            }
            other => panic!("expected Extracted, got {other:?}"),
        }
    }

    #[test]
    fn extract_enricher_fallback_does_not_stop_the_chain() {
        let mut registry = Registry::new();
        registry
            .register(
                ScriptedExtractor::new("enricher", enrich_caps())
                    .extracting(|_| {
                        ExtractOutcome::Fallback(HisterError::Extraction(
                            "enrich skipped".to_string(),
                        ))
                    })
                    .boxed(),
            )
            .unwrap();
        registry
            .register(
                ScriptedExtractor::new("extractor", extract_caps())
                    .extracting(|_| {
                        ExtractOutcome::Extracted(Document::new("https://example.com/ok"))
                    })
                    .boxed(),
            )
            .unwrap();

        match registry.extract(&doc()) {
            ExtractOutcome::Extracted(result) => assert_eq!(result.url, "https://example.com/ok"),
            other => panic!("expected Extracted, got {other:?}"),
        }
    }

    #[test]
    fn extract_with_no_matching_extractor_returns_fallback() {
        let registry = Registry::new();
        match registry.extract(&doc()) {
            ExtractOutcome::Fallback(_) => {}
            other => panic!("expected Fallback, got {other:?}"),
        }
    }

    #[test]
    fn preview_with_no_starting_point_starts_from_the_beginning() {
        let mut registry = Registry::new();
        registry
            .register(
                ScriptedExtractor::new("first", preview_caps())
                    .previewing(|_| PreviewOutcome::Previewed(PreviewResponse::default()))
                    .boxed(),
            )
            .unwrap();

        match registry.preview(&doc(), None) {
            PreviewOutcome::Previewed(_) => {}
            other => panic!("expected Previewed, got {other:?}"),
        }
    }

    #[test]
    fn preview_starting_point_skips_ahead_but_keeps_fallback_chain() {
        let mut registry = Registry::new();
        registry
            .register(
                ScriptedExtractor::new("skipped", preview_caps())
                    .previewing(|_| panic!("must not run: before the starting point"))
                    .boxed(),
            )
            .unwrap();
        registry
            .register(
                ScriptedExtractor::new("start-here", preview_caps())
                    .previewing(|_| {
                        PreviewOutcome::Fallback(HisterError::Extraction("try next".to_string()))
                    })
                    .boxed(),
            )
            .unwrap();
        registry
            .register(
                ScriptedExtractor::new("fallback-target", preview_caps())
                    .previewing(|_| PreviewOutcome::Previewed(PreviewResponse::default()))
                    .boxed(),
            )
            .unwrap();

        match registry.preview(&doc(), Some("start-here")) {
            PreviewOutcome::Previewed(_) => {}
            other => panic!("expected Previewed, got {other:?}"),
        }
    }

    #[test]
    fn preview_unknown_starting_point_is_a_hard_error() {
        let registry = Registry::new();
        match registry.preview(&doc(), Some("does-not-exist")) {
            PreviewOutcome::Abort(HisterError::InvalidConfig(_)) => {}
            other => panic!("expected Abort(InvalidConfig), got {other:?}"),
        }
    }

    #[test]
    fn preview_disabled_starting_point_is_a_hard_error() {
        let mut registry = Registry::new();
        registry
            .register(
                ScriptedExtractor::new("disabled", preview_caps())
                    .disabled()
                    .boxed(),
            )
            .unwrap();
        match registry.preview(&doc(), Some("disabled")) {
            PreviewOutcome::Abort(HisterError::InvalidConfig(_)) => {}
            other => panic!("expected Abort(InvalidConfig), got {other:?}"),
        }
    }

    #[test]
    fn preview_non_preview_capable_starting_point_is_a_hard_error() {
        let mut registry = Registry::new();
        registry
            .register(ScriptedExtractor::new("extract-only", extract_caps()).boxed())
            .unwrap();
        match registry.preview(&doc(), Some("extract-only")) {
            PreviewOutcome::Abort(HisterError::InvalidConfig(_)) => {}
            other => panic!("expected Abort(InvalidConfig), got {other:?}"),
        }
    }

    #[test]
    fn preview_non_matching_starting_point_is_a_hard_error() {
        let mut registry = Registry::new();
        registry
            .register(
                ScriptedExtractor::new("non-matching", preview_caps())
                    .non_matching()
                    .boxed(),
            )
            .unwrap();
        match registry.preview(&doc(), Some("non-matching")) {
            PreviewOutcome::Abort(HisterError::InvalidConfig(_)) => {}
            other => panic!("expected Abort(InvalidConfig), got {other:?}"),
        }
    }

    #[test]
    fn apply_configs_matches_names_case_insensitively_and_ignores_unknown() {
        let mut registry = Registry::new();
        registry
            .register(ScriptedExtractor::new("Reddit", extract_caps()).boxed())
            .unwrap();

        let mut configs = HashMap::new();
        let disabled_config = ExtractorConfig {
            enabled: false,
            ..Default::default()
        };
        configs.insert("reddit".to_string(), disabled_config);
        configs.insert("unregistered".to_string(), ExtractorConfig::default());

        registry.apply_configs(configs).unwrap();

        assert!(!registry.list().next().unwrap().config().enabled);
    }

    #[test]
    fn list_matching_only_returns_enabled_matching_names() {
        let mut registry = Registry::new();
        registry
            .register(ScriptedExtractor::new("matches", extract_caps()).boxed())
            .unwrap();
        registry
            .register(
                ScriptedExtractor::new("non-matching", extract_caps())
                    .non_matching()
                    .boxed(),
            )
            .unwrap();
        registry
            .register(
                ScriptedExtractor::new("disabled", extract_caps())
                    .disabled()
                    .boxed(),
            )
            .unwrap();

        assert_eq!(registry.list_matching(&doc()), vec!["matches"]);
    }

    #[test]
    fn list_matching_preview_only_returns_preview_capable_names() {
        let mut registry = Registry::new();
        registry
            .register(ScriptedExtractor::new("previewable", preview_caps()).boxed())
            .unwrap();
        registry
            .register(ScriptedExtractor::new("extract-only", extract_caps()).boxed())
            .unwrap();

        assert_eq!(registry.list_matching_preview(&doc()), vec!["previewable"]);
    }
}
