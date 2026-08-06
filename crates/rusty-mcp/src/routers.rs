//! Paginating replacements for `rmcp`'s tool and prompt handler macros.
//!
//! `rmcp` routes tools and prompts for you, but neither generated `list`
//! method pages: both take a `PaginatedRequestParams`, discard it, return
//! `list_all()` and set `next_cursor: None`. The server therefore advertises
//! that it accepts a cursor and then ignores it, so a client that pages sees a
//! full first page with no cursor and concludes it has everything. At a few
//! dozen tools that is invisible; at several hundred it truncates silently
//! from the client's side.
//!
//! [`crate::resources::ResourceRegistry`] already pages. These macros bring
//! tools and prompts to the same place, on the same cursor.
//!
//! ```ignore
//! impl ServerHandler for MyServer {
//!     fn get_info(&self) -> ServerInfo { /* ... */ }
//!
//!     rusty_mcp::forward_tool_methods!(tool_router);
//!     rusty_mcp::forward_prompt_methods!(prompt_router);
//! }
//! ```
//!
//! # These replace `#[tool_handler]` and `#[prompt_handler]`
//!
//! Not composed with them — instead of them. Two separate reasons, both
//! discovered the hard way, and both silent if you get it wrong:
//!
//! - **`#[prompt_handler]` overwrites an override.** It scans the impl for
//!   `list_prompts` and replaces its body with the generated one. A
//!   hand-written paginating version compiles, runs, and is thrown away.
//! - **`#[tool_handler]` cannot see one.** It guards with
//!   `if !has_method("list_tools", ...)`, which sounds like an override would
//!   work — but attribute macros expand *before* `macro_rules!` invocations
//!   inside the item, so all it sees is an unexpanded macro call, not a
//!   function. It generates its own `list_tools`, and you get `E0201:
//!   duplicate definitions`.
//!
//! The second is at least a compile error. The first is not, which is why
//! neither attribute is worth keeping alongside these.
//!
//! You therefore write `get_info` yourself, which
//! [`crate::server_info`] already exists to make a one-liner.

/// Serve `tools/call`, a paginated `tools/list`, and `get_tool` from a
/// `ToolRouter` field.
///
/// Use **instead of** `#[tool_handler]`. Optionally takes a page size;
/// defaults to [`crate::pagination::DEFAULT_PAGE_SIZE`].
///
/// ```ignore
/// rusty_mcp::forward_tool_methods!(tool_router);
/// rusty_mcp::forward_tool_methods!(tool_router, 25);
/// ```
#[macro_export]
macro_rules! forward_tool_methods {
    ($field:ident) => {
        $crate::forward_tool_methods!($field, $crate::pagination::DEFAULT_PAGE_SIZE);
    };
    ($field:ident, $page_size:expr) => {
        async fn call_tool(
            &self,
            request: $crate::__private::CallToolRequestParams,
            context: $crate::__private::RequestContext<$crate::__private::RoleServer>,
        ) -> ::core::result::Result<
            $crate::__private::CallToolResponse,
            $crate::__private::ErrorData,
        > {
            let call = $crate::__private::ToolCallContext::new(self, request, context);
            self.$field.call(call).await
        }

        async fn list_tools(
            &self,
            request: ::core::option::Option<$crate::__private::PaginatedRequestParams>,
            context: $crate::__private::RequestContext<$crate::__private::RoleServer>,
        ) -> ::core::result::Result<$crate::__private::ListToolsResult, $crate::__private::ErrorData>
        {
            let cursor = request.as_ref().and_then(|r| r.cursor.as_deref());
            let all = self.$field.list_all();

            let (tools, next) = $crate::pagination::page_owned(
                &all,
                |tool| tool.name.as_ref(),
                $crate::pagination::CursorKind::Tool,
                cursor,
                $page_size,
            )?;

            let mut result = $crate::__private::ListToolsResult::with_all_items(tools);
            result.next_cursor = next;
            $crate::__private::apply_cache_hints(
                &context,
                &mut result.ttl_ms,
                &mut result.cache_scope,
            );
            ::core::result::Result::Ok(result)
        }

        fn get_tool(&self, name: &str) -> ::core::option::Option<$crate::__private::Tool> {
            self.$field.get(name).cloned()
        }
    };
}

/// Serve `prompts/get` and a paginated `prompts/list` from a `PromptRouter`
/// field.
///
/// Use **instead of** `#[prompt_handler]`, which would overwrite the
/// `list_prompts` this generates.
///
/// ```ignore
/// rusty_mcp::forward_prompt_methods!(prompt_router);
/// rusty_mcp::forward_prompt_methods!(prompt_router, 25);
/// ```
#[macro_export]
macro_rules! forward_prompt_methods {
    ($field:ident) => {
        $crate::forward_prompt_methods!($field, $crate::pagination::DEFAULT_PAGE_SIZE);
    };
    ($field:ident, $page_size:expr) => {
        async fn get_prompt(
            &self,
            request: $crate::__private::GetPromptRequestParams,
            context: $crate::__private::RequestContext<$crate::__private::RoleServer>,
        ) -> ::core::result::Result<
            $crate::__private::GetPromptResponse,
            $crate::__private::ErrorData,
        > {
            let prompt_context = $crate::__private::PromptContext::new(
                self,
                request.name,
                request.arguments,
                context,
            );
            self.$field.get_prompt(prompt_context).await
        }

        async fn list_prompts(
            &self,
            request: ::core::option::Option<$crate::__private::PaginatedRequestParams>,
            context: $crate::__private::RequestContext<$crate::__private::RoleServer>,
        ) -> ::core::result::Result<
            $crate::__private::ListPromptsResult,
            $crate::__private::ErrorData,
        > {
            let cursor = request.as_ref().and_then(|r| r.cursor.as_deref());
            let all = self.$field.list_all();

            let (prompts, next) = $crate::pagination::page_owned(
                &all,
                |prompt| prompt.name.as_ref(),
                $crate::pagination::CursorKind::Prompt,
                cursor,
                $page_size,
            )?;

            let mut result = $crate::__private::ListPromptsResult::with_all_items(prompts);
            result.next_cursor = next;
            $crate::__private::apply_cache_hints(
                &context,
                &mut result.ttl_ms,
                &mut result.cache_scope,
            );
            ::core::result::Result::Ok(result)
        }
    };
}
