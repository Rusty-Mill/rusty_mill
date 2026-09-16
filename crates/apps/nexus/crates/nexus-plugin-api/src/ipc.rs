//! IPC dispatch abstraction — stable plugin-facing interface.
//!
//! # Caller identity
//!
//! [`IpcDispatcher::dispatch`]/[`dispatch_async`](IpcDispatcher::dispatch_async)
//! take a kernel-verified `caller_plugin_id` — the identity of the plugin
//! that issued this call, bound by the kernel/host boundary (e.g.
//! `KernelPluginContext`'s own `plugin_id`, or the WASM sandbox's
//! `PluginData::plugin_id`), never by caller-supplied JSON. Handlers whose
//! business logic depends on *who* is calling (not just *what* they hold
//! capabilities for) must derive that identity from here, not from an
//! args field a caller can set to anything.
//!
//! Threading `caller_plugin_id` through every `CorePlugin::dispatch` /
//! `dispatch_async` implementation as well would tax the near-totality of
//! handlers that never need it to serve the few that do (the same
//! trade-off `nexus-kernel`'s `IPC_CANCEL` task-local makes for
//! cancellation). Instead, an `IpcDispatcher` implementation that routes
//! into a `CorePlugin` backend binds the verified id via [`scope_caller`]
//! around the handler invocation; handlers that care read it back via
//! [`ipc_caller_plugin_id`].

use std::future::Future;
use std::pin::Pin;

use crate::capability::Capability;
use crate::error::IpcError;

/// A boxed, `'static`, `Send` future returned by an async IPC handler.
pub type IpcFuture = Pin<Box<dyn Future<Output = Result<serde_json::Value, IpcError>> + Send>>;

/// Dispatches an IPC command to a loaded plugin's handler.
///
/// The caller's capability check is performed by the kernel context before
/// delegating here; implementations only resolve the target and invoke the
/// handler.
pub trait IpcDispatcher: Send + Sync {
    /// Dispatch `command_id` on plugin `target_plugin_id` with `args`.
    ///
    /// `caller_plugin_id` is the kernel-verified identity of the plugin
    /// issuing this call (see the module docs) — implementations that
    /// delegate to a `CorePlugin` backend should bind it via
    /// [`scope_caller`] around the handler invocation.
    ///
    /// # Errors
    /// - [`IpcError::PluginNotFound`] if the target plugin is not loaded.
    /// - [`IpcError::CommandNotFound`] if the target does not register that command.
    /// - [`IpcError::PluginCrashedDuringCall`] on panic or execution error.
    fn dispatch(
        &self,
        caller_plugin_id: &str,
        target_plugin_id: &str,
        command_id: &str,
        args: &serde_json::Value,
    ) -> Result<serde_json::Value, IpcError>;

    /// Try to dispatch `command_id` asynchronously.
    ///
    /// Returns `Some(future)` when the target plugin has an async handler for
    /// this command; returns `None` when it is sync-only — the caller should
    /// then fall back to [`dispatch`](IpcDispatcher::dispatch).
    fn dispatch_async(
        &self,
        caller_plugin_id: &str,
        target_plugin_id: &str,
        command_id: &str,
        args: serde_json::Value,
    ) -> Option<IpcFuture> {
        let _ = (caller_plugin_id, target_plugin_id, command_id, args);
        None
    }

    /// Capabilities the caller must hold to invoke `command_id` on
    /// `target_plugin_id`, in addition to the unconditional
    /// [`Capability::IpcCall`] check the kernel context performs first.
    ///
    /// The default returns an empty list — most IPC commands need nothing
    /// beyond `IpcCall`. Override this for commands that perform high-impact
    /// side effects (process spawn, external network, …) so callers must
    /// hold the matching kernel capability rather than laundering the
    /// effect through `IpcCall` alone. See issue #77.
    ///
    /// [`KernelPluginContext::ipc_call`] consults
    /// [`Self::required_caller_caps_for_args`] (which defaults to this
    /// method) **before** dispatch, so handlers themselves don't need
    /// to re-check.
    fn required_caller_caps(&self, target_plugin_id: &str, command_id: &str) -> Vec<Capability> {
        let _ = (target_plugin_id, command_id);
        Vec::new()
    }

    /// Args-aware extension of [`Self::required_caller_caps`]
    /// (ADR 0022 Phase 2). Lets dispatchers tighten the caller-cap
    /// requirement based on what the call is asking for — e.g.
    /// `com.nexus.ai::stream_chat` requires `ai.tools.write` only
    /// when `tools=auto` advertises mutating tools.
    ///
    /// The default delegates to the args-less form so existing
    /// implementations and dispatch tables keep their shape;
    /// implementors that want args-aware policies override this
    /// directly and may ignore the static fallback.
    fn required_caller_caps_for_args(
        &self,
        target_plugin_id: &str,
        command_id: &str,
        args: &serde_json::Value,
    ) -> Vec<Capability> {
        let _ = args;
        self.required_caller_caps(target_plugin_id, command_id)
    }

    /// P1-02 — `true` if `command_id` on `target_plugin_id` is marked
    /// "in-tree only" in the cap matrix (i.e. only callable by a
    /// context whose `trust_level == Core`). Distinct from the
    /// capability gate: a community plugin that holds every cap
    /// listed in [`Self::required_caller_caps`] for this handler is
    /// still rejected if this returns `true`.
    ///
    /// Useful for handlers that expose host secrets / mutate
    /// trust-establishing state and must never be reachable from a
    /// sandboxed plugin no matter what caps it accumulates
    /// (`com.nexus.ai::resolve_credentials` is the seed caller).
    ///
    /// Default `false` — the historical contract is "cap-gated only".
    fn is_handler_internal_only(&self, target_plugin_id: &str, command_id: &str) -> bool {
        let _ = (target_plugin_id, command_id);
        false
    }
}

// ── Verified caller identity ────────────────────────────────────────────────
//
// See the module-level "Caller identity" docs above for the rationale
// (mirrors `nexus-kernel`'s `IPC_CANCEL` task-local: a thread-local scope
// rather than a `CorePlugin::dispatch` parameter, so the ~30 unrelated
// `CorePlugin` implementors never see it).
//
// Plain `thread_local!`, not `tokio::task_local!`: every production
// `IpcDispatcher::dispatch` call into a `CorePlugin` backend is fully
// synchronous on the calling thread (the backend mutex is held across the
// call), so there is no await point across which the binding would need to
// survive a task being moved to a different worker thread.

use std::cell::RefCell;

thread_local! {
    static IPC_CALLER: RefCell<Option<String>> = const { RefCell::new(None) };
}

/// Return the verified identity of the plugin that initiated the IPC call
/// currently being dispatched on this thread, if any.
///
/// Bound by an [`IpcDispatcher`] implementation via [`scope_caller`] around
/// the call into a `CorePlugin`'s `dispatch`/`dispatch_async`. Handlers
/// that must not trust a caller-supplied identity in `args` (e.g. a
/// per-plugin secret-vault namespace) should call this instead.
///
/// Returns `None` outside an active scoped dispatch — e.g. a unit test
/// that calls `CorePlugin::dispatch` directly, or a host-side entry point
/// (CLI `plugin call`) that never went through an `IpcDispatcher`.
#[must_use]
pub fn ipc_caller_plugin_id() -> Option<String> {
    IPC_CALLER.with(|c| c.borrow().clone())
}

/// Run `f` with `caller_plugin_id` bound as the active verified IPC caller
/// identity for the duration of the call. Restores whatever was previously
/// bound afterward (including on panic/unwind), so nested/reentrant
/// dispatch chains each observe their own immediate caller.
///
/// Called by `IpcDispatcher` implementations that route into a
/// `CorePlugin` backend. Plugin/handler code should not call this
/// directly — it reads the binding via [`ipc_caller_plugin_id`].
pub fn scope_caller<F, T>(caller_plugin_id: &str, f: F) -> T
where
    F: FnOnce() -> T,
{
    struct Restore(Option<String>);
    impl Drop for Restore {
        fn drop(&mut self) {
            IPC_CALLER.with(|c| *c.borrow_mut() = self.0.take());
        }
    }
    let previous = IPC_CALLER.with(|c| c.replace(Some(caller_plugin_id.to_string())));
    let _restore = Restore(previous);
    f()
}

#[cfg(test)]
mod caller_tests {
    use super::{ipc_caller_plugin_id, scope_caller};

    #[test]
    fn none_outside_scope() {
        assert_eq!(ipc_caller_plugin_id(), None);
    }

    #[test]
    fn bound_inside_scope_and_restored_after() {
        assert_eq!(ipc_caller_plugin_id(), None);
        let seen = scope_caller("com.test.caller", ipc_caller_plugin_id);
        assert_eq!(seen, Some("com.test.caller".to_string()));
        assert_eq!(ipc_caller_plugin_id(), None);
    }

    #[test]
    fn nested_scopes_restore_the_outer_caller() {
        scope_caller("outer", || {
            assert_eq!(ipc_caller_plugin_id(), Some("outer".to_string()));
            scope_caller("inner", || {
                assert_eq!(ipc_caller_plugin_id(), Some("inner".to_string()));
            });
            assert_eq!(ipc_caller_plugin_id(), Some("outer".to_string()));
        });
    }
}
