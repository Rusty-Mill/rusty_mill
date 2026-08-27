//! `$/progress` and work-done-progress types (LSP 3.15+).
//!
//! A server reports progress on a long-running operation (indexing, a slow
//! computation) by reserving a token via `window/workDoneProgress/create`,
//! then sending a `begin` / any number of `report` / one `end` sequence of
//! `$/progress` notifications carrying that token. [`crate::Client`] exposes
//! typed helpers for both halves of this handshake
//! ([`create_progress`](crate::Client::create_progress),
//! [`progress_begin`](crate::Client::progress_begin),
//! [`progress_report`](crate::Client::progress_report),
//! [`progress_end`](crate::Client::progress_end)).

use serde::{Deserialize, Serialize};

/// Identifies one progress sequence, chosen by whichever side creates it.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ProgressToken {
    /// Numeric token.
    Number(i64),
    /// String token.
    String(String),
}

impl From<i64> for ProgressToken {
    fn from(n: i64) -> Self {
        ProgressToken::Number(n)
    }
}

impl From<String> for ProgressToken {
    fn from(s: String) -> Self {
        ProgressToken::String(s)
    }
}

impl From<&str> for ProgressToken {
    fn from(s: &str) -> Self {
        ProgressToken::String(s.to_owned())
    }
}

/// Parameters of the `$/progress` notification.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProgressParams {
    /// The token identifying the progress sequence this update belongs to.
    pub token: ProgressToken,
    /// The progress payload.
    pub value: WorkDoneProgress,
}

/// One update in a work-done-progress sequence: exactly one
/// [`WorkDoneProgress::Begin`], any number of [`WorkDoneProgress::Report`],
/// then exactly one [`WorkDoneProgress::End`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum WorkDoneProgress {
    /// Starts a progress sequence.
    Begin(WorkDoneProgressBegin),
    /// Reports incremental progress within a sequence.
    Report(WorkDoneProgressReport),
    /// Ends a progress sequence.
    End(WorkDoneProgressEnd),
}

/// Starts a work-done-progress sequence.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkDoneProgressBegin {
    /// A short title for the operation.
    pub title: String,
    /// Whether the client may offer a cancel button for this operation. A
    /// server that supports cancellation should watch for
    /// `window/workDoneProgress/cancel` (see
    /// [`LanguageServer::work_done_progress_cancel`](crate::LanguageServer::work_done_progress_cancel)).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cancellable: Option<bool>,
    /// A short, user-facing progress message.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    /// Progress percentage, `0..=100`. Omit if the total work isn't known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub percentage: Option<u32>,
}

/// Reports incremental progress within a sequence.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkDoneProgressReport {
    /// Whether the client may offer a cancel button for this operation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cancellable: Option<bool>,
    /// A short, user-facing progress message.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    /// Progress percentage, `0..=100`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub percentage: Option<u32>,
}

/// Ends a work-done-progress sequence.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkDoneProgressEnd {
    /// A final, user-facing message.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

/// Parameters of the `window/workDoneProgress/create` request: asks the
/// client to reserve `token` for a subsequent progress sequence.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkDoneProgressCreateParams {
    /// The token to reserve.
    pub token: ProgressToken,
}

/// Parameters of the `window/workDoneProgress/cancel` notification: the
/// client telling the server the user cancelled the operation behind
/// `token`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkDoneProgressCancelParams {
    /// The token identifying the operation to cancel.
    pub token: ProgressToken,
}

/// Mixin flattened into a request's params to opt it into work-done
/// progress: the client may pre-create `work_done_token` itself (instead of
/// the server calling `window/workDoneProgress/create`) and expect the
/// server to report against it via
/// [`Client::progress_begin`](crate::Client::progress_begin) /
/// [`progress_report`](crate::Client::progress_report) /
/// [`progress_end`](crate::Client::progress_end).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkDoneProgressParams {
    /// The client-supplied token to report progress against, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub work_done_token: Option<ProgressToken>,
}

/// The server-capability counterpart of [`WorkDoneProgressParams`]: mixed
/// into a `ServerCapabilities` provider-options struct (e.g.
/// [`crate::lsp::HoverProviderCapability`]'s `Options` variant) to advertise
/// that the provider reports work-done progress. Several capability options
/// the spec defines add nothing beyond this single field, so this one struct
/// is shared across all of them rather than duplicated per method.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkDoneProgressOptions {
    /// Whether the server reports work-done progress for this provider.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub work_done_progress: Option<bool>,
}

/// Mixin flattened into a "list" request's params to opt it into streaming
/// partial results: when `partial_result_token` is set, the server may send
/// zero or more `$/progress` notifications on that token — each carrying a
/// chunk of the result, via
/// [`Client::send_progress`](crate::Client::send_progress) — before (or
/// instead of) returning the final response.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PartialResultParams {
    /// The client-supplied token to stream partial results on, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub partial_result_token: Option<ProgressToken>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn begin_report_end_serialize_with_kind_tag() {
        let begin = WorkDoneProgress::Begin(WorkDoneProgressBegin {
            title: "Indexing".to_owned(),
            cancellable: Some(true),
            message: None,
            percentage: Some(0),
        });
        assert_eq!(
            serde_json::to_value(&begin).unwrap(),
            json!({"kind": "begin", "title": "Indexing", "cancellable": true, "percentage": 0})
        );

        let end = WorkDoneProgress::End(WorkDoneProgressEnd {
            message: Some("done".to_owned()),
        });
        assert_eq!(
            serde_json::to_value(&end).unwrap(),
            json!({"kind": "end", "message": "done"})
        );
    }

    #[test]
    fn progress_token_is_untagged_number_or_string() {
        assert_eq!(
            serde_json::to_value(ProgressToken::Number(1)).unwrap(),
            json!(1)
        );
        assert_eq!(
            serde_json::to_value(ProgressToken::String("a".into())).unwrap(),
            json!("a")
        );
        let token: ProgressToken = serde_json::from_value(json!(42)).unwrap();
        assert_eq!(token, ProgressToken::Number(42));
    }

    #[test]
    fn progress_param_mixins_flatten_camel_case_and_omit_when_absent() {
        assert_eq!(
            serde_json::to_value(WorkDoneProgressParams::default()).unwrap(),
            json!({})
        );
        assert_eq!(
            serde_json::to_value(WorkDoneProgressParams {
                work_done_token: Some(ProgressToken::from("t1")),
            })
            .unwrap(),
            json!({"workDoneToken": "t1"})
        );
        assert_eq!(
            serde_json::to_value(PartialResultParams {
                partial_result_token: Some(ProgressToken::from(2)),
            })
            .unwrap(),
            json!({"partialResultToken": 2})
        );
    }

    #[test]
    fn work_done_progress_options_omits_when_absent() {
        assert_eq!(
            serde_json::to_value(WorkDoneProgressOptions::default()).unwrap(),
            json!({})
        );
        assert_eq!(
            serde_json::to_value(WorkDoneProgressOptions {
                work_done_progress: Some(true),
            })
            .unwrap(),
            json!({"workDoneProgress": true})
        );
    }
}
