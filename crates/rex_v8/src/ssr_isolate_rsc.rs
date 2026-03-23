//! RSC, MCP tool, and server action methods for [`SsrIsolate`].
//!
//! Split out from `ssr_isolate.rs` to stay under the 700-line file limit.

use anyhow::{Context, Result};

use crate::ssr_isolate::{RscRenderResult, SsrIsolate};

impl SsrIsolate {
    /// Load RSC bundles (flight + SSR) into the V8 context.
    ///
    /// Both bundles are IIFEs evaluated sequentially in the same context.
    /// The flight bundle sets `__rex_render_flight`, `__rex_render_rsc_to_html`, etc.
    /// The SSR bundle sets `__rex_rsc_flight_to_html`, `__rex_resolve_ssr_pending`, etc.
    pub fn load_rsc_bundles(&mut self, flight_bundle_js: &str, ssr_bundle_js: &str) -> Result<()> {
        {
            v8::scope_with_context!(scope, &mut self.isolate, &self.context);

            // Evaluate the flight bundle (sets __rex_render_flight, etc.)
            v8_eval!(scope, flight_bundle_js, "rsc-server-bundle.js")
                .context("Failed to evaluate RSC flight bundle")?;

            // Evaluate the SSR bundle (sets __rex_rsc_flight_to_html, etc.)
            v8_eval!(scope, ssr_bundle_js, "rsc-ssr-bundle.js")
                .context("Failed to evaluate RSC SSR bundle")?;

            // Re-lookup RSC functions now that both bundles are loaded
            let ctx = scope.get_current_context();
            let global = ctx.global(scope);

            self.rsc_flight_fn = v8_get_optional_fn!(scope, global, "__rex_render_flight");
            self.rsc_to_html_fn = v8_get_optional_fn!(scope, global, "__rex_render_rsc_to_html");
            self.server_action_fn = v8_get_optional_fn!(scope, global, "__rex_call_server_action");
            self.server_action_encoded_fn =
                v8_get_optional_fn!(scope, global, "__rex_call_server_action_encoded");
            self.form_action_fn = v8_get_optional_fn!(scope, global, "__rex_call_form_action");
        }

        tracing::debug!("RSC bundles loaded into V8 context");
        Ok(())
    }

    /// Render RSC flight data for a route (app/ routes only).
    /// Returns the flight data string for client-side navigation.
    /// Handles async server components via iterative resolve loop.
    pub fn render_rsc_flight(&mut self, route_key: &str, props_json: &str) -> Result<String> {
        let rsc_fn = self
            .rsc_flight_fn
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("RSC flight renderer not loaded"))?;

        let result_str = {
            v8::scope_with_context!(scope, &mut self.isolate, &self.context);

            let func = v8::Local::new(scope, rsc_fn);
            let undef = v8::undefined(scope);
            let arg0 = v8::String::new(scope, route_key)
                .ok_or_else(|| anyhow::anyhow!("V8 string alloc failed"))?;
            let arg1 = v8::String::new(scope, props_json)
                .ok_or_else(|| anyhow::anyhow!("V8 string alloc failed"))?;

            let result = v8_call!(scope, func, undef.into(), &[arg0.into(), arg1.into()])
                .map_err(|e| anyhow::anyhow!("RSC flight render error: {e}"))?;

            result.to_rust_string_lossy(scope)
        };

        // Run fetch loop in case server components used fetch()
        crate::fetch::run_fetch_loop(&mut self.isolate, &self.context);

        if result_str == "__REX_RSC_ASYNC__" {
            self.resolve_rsc_async()?;

            let finalized = {
                v8::scope_with_context!(scope, &mut self.isolate, &self.context);
                let result = v8_eval!(
                    scope,
                    "globalThis.__rex_finalize_rsc_flight()",
                    "<rsc-finalize-flight>"
                )
                .map_err(|e| anyhow::anyhow!("RSC finalize error: {e}"))?;
                result.to_rust_string_lossy(scope)
            };
            return Ok(finalized);
        }

        Ok(result_str)
    }

    /// Two-pass RSC render: flight data + HTML (app/ routes only).
    /// Returns RenderResult with body HTML, head, and flight data.
    /// Handles async server components via iterative resolve loop.
    pub fn render_rsc_to_html(
        &mut self,
        route_key: &str,
        props_json: &str,
    ) -> Result<RscRenderResult> {
        let rsc_fn = self
            .rsc_to_html_fn
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("RSC HTML renderer not loaded"))?;

        let result_str = {
            v8::scope_with_context!(scope, &mut self.isolate, &self.context);

            let func = v8::Local::new(scope, rsc_fn);
            let undef = v8::undefined(scope);
            let arg0 = v8::String::new(scope, route_key)
                .ok_or_else(|| anyhow::anyhow!("V8 string alloc failed"))?;
            let arg1 = v8::String::new(scope, props_json)
                .ok_or_else(|| anyhow::anyhow!("V8 string alloc failed"))?;

            let result = v8_call!(scope, func, undef.into(), &[arg0.into(), arg1.into()])
                .map_err(|e| anyhow::anyhow!("RSC render error: {e}"))?;

            result.to_rust_string_lossy(scope)
        };

        // Run fetch loop in case server components used fetch()
        crate::fetch::run_fetch_loop(&mut self.isolate, &self.context);

        // Check for sentinel values before JSON parsing
        if result_str == "__REX_NOT_FOUND__" {
            return Err(anyhow::anyhow!("__REX_NOT_FOUND__"));
        }
        if let Some(rest) = result_str.strip_prefix("__REX_REDIRECT__:") {
            return Err(anyhow::anyhow!("__REX_REDIRECT__:{rest}"));
        }

        if result_str == "__REX_RSC_HTML_ASYNC__" {
            self.resolve_rsc_async()?;

            let finalized = {
                v8::scope_with_context!(scope, &mut self.isolate, &self.context);

                let result = v8_eval!(
                    scope,
                    "globalThis.__rex_finalize_rsc_to_html()",
                    "<rsc-finalize-html>"
                )
                .map_err(|e| anyhow::anyhow!("RSC finalize error: {e}"))?;
                result.to_rust_string_lossy(scope)
            };

            // Check for sentinel values in async result
            if finalized == "__REX_NOT_FOUND__" {
                return Err(anyhow::anyhow!("__REX_NOT_FOUND__"));
            }
            if let Some(rest) = finalized.strip_prefix("__REX_REDIRECT__:") {
                return Err(anyhow::anyhow!("__REX_REDIRECT__:{rest}"));
            }

            // React's flight protocol can produce lone surrogates as JSON hex
            // escapes (\uD800-\uDFFF) which serde_json rejects. Replace them
            // with the Unicode replacement character escape (\uFFFD).
            let finalized = sanitize_json_surrogates(&finalized);
            let parsed: RscRenderResult = match serde_json::from_str(&finalized) {
                Ok(v) => v,
                Err(e) => {
                    tracing::error!(
                        len = finalized.len(),
                        err = %e,
                        line = e.line(),
                        col = e.column(),
                        "Failed to parse RSC finalize result"
                    );
                    return Err(anyhow::anyhow!("Failed to parse RSC finalize result: {e}"));
                }
            };
            return Ok(parsed);
        }

        let parsed: RscRenderResult =
            serde_json::from_str(&result_str).context("Failed to parse RSC render result")?;
        Ok(parsed)
    }

    /// Iterative resolve loop for async server components.
    /// Runs fetch loop + microtask pump, then calls __rex_resolve_rsc_pending()
    /// until all async slots are resolved (or timeout).
    fn resolve_rsc_async(&mut self) -> Result<()> {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            if std::time::Instant::now() > deadline {
                return Err(anyhow::anyhow!("RSC async resolution timed out after 10s"));
            }

            // Process any pending I/O (fetch, TCP sockets)
            crate::fetch::run_fetch_loop(&mut self.isolate, &self.context);

            // Pump microtasks and timers to full convergence.
            // run_fetch_loop exits when there is no I/O work, but React's
            // internal rendering may still need microtask rounds (promise
            // chains, ReadableStream chunk delivery, setTimeout(0) scheduling)
            // to fully resolve async server components and close the flight
            // data stream.
            self.drain_microtasks_and_timers();

            let status = {
                v8::scope_with_context!(scope, &mut self.isolate, &self.context);
                let result = v8_eval!(
                    scope,
                    "globalThis.__rex_resolve_rsc_pending()",
                    "<rsc-resolve>"
                )
                .map_err(|e| anyhow::anyhow!("RSC resolve error: {e}"))?;
                result.to_rust_string_lossy(scope)
            };

            match status.as_str() {
                "done" => {
                    break;
                }
                "pending" => {
                    // Yield briefly to avoid CPU-spinning when async slots are
                    // waiting on external I/O (fetch, timers with real delays).
                    std::thread::sleep(std::time::Duration::from_millis(1));
                    continue;
                }
                other => {
                    return Err(anyhow::anyhow!("Unexpected RSC resolve status: {}", other));
                }
            }
        }
        Ok(())
    }

    /// Pump V8 microtasks and JS timers until no more progress is made.
    ///
    /// This ensures React's internal rendering state (ReadableStream chunk
    /// delivery, promise chains from async server components, SSR pass
    /// promises) fully converges after I/O operations complete. Without
    /// this, the flight data stream may not have closed yet when we check
    /// `__rex_resolve_rsc_pending()`, leading to incomplete flight data
    /// being embedded in the HTML response.
    fn drain_microtasks_and_timers(&mut self) {
        // Cap iterations to prevent infinite loops from buggy JS timer chains.
        for _ in 0..1000 {
            self.isolate.perform_microtask_checkpoint();

            let timer_fired = crate::fetch::drain_js_timers(&mut self.isolate, &self.context);
            if timer_fired {
                // Timer callbacks may have queued microtasks — drain them,
                // then check for more expired timers.
                self.isolate.perform_microtask_checkpoint();
                continue;
            }

            // If microtask processing produced new fetch requests, stop
            // here — the caller's next run_fetch_loop iteration will
            // handle them.
            let has_new_fetch = crate::fetch::FETCH_QUEUE.with(|q| !q.borrow().is_empty());
            if has_new_fetch {
                break;
            }

            // Run one more microtask checkpoint — the previous checkpoint
            // may have resolved promises that schedule MessageChannel
            // callbacks (used by React's scheduler), which in our polyfill
            // become microtasks. Without this second pass, the scheduler
            // callback fires but its follow-up work is deferred to the
            // next resolve_rsc_async iteration, adding unnecessary latency.
            self.isolate.perform_microtask_checkpoint();
            let timer_fired2 = crate::fetch::drain_js_timers(&mut self.isolate, &self.context);
            if timer_fired2 {
                continue;
            }

            // Converged — no more timers, no new fetch, microtasks drained.
            break;
        }
    }

    /// List registered MCP tools. Returns JSON array of {name, description, parameters}.
    /// Returns Ok(None) if no MCP tools are loaded.
    pub fn list_mcp_tools(&mut self) -> Result<Option<String>> {
        let list_fn = match &self.mcp_list_fn {
            Some(f) => f,
            None => return Ok(None),
        };

        v8::scope_with_context!(scope, &mut self.isolate, &self.context);

        let func = v8::Local::new(scope, list_fn);
        let undef = v8::undefined(scope);

        let result = v8_call!(scope, func, undef.into(), &[])
            .map_err(|e| anyhow::anyhow!("MCP list error: {e}"))?;

        Ok(Some(result.to_rust_string_lossy(scope)))
    }

    /// Call an MCP tool by name with JSON parameters. Returns JSON result.
    /// Handles async tool handlers by pumping V8's microtask queue.
    pub fn call_mcp_tool(&mut self, name: &str, params_json: &str) -> Result<String> {
        let call_fn = self
            .mcp_call_fn
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("MCP tools not loaded"))?;

        let result_str = {
            v8::scope_with_context!(scope, &mut self.isolate, &self.context);

            let func = v8::Local::new(scope, call_fn);
            let undef = v8::undefined(scope);
            let arg0 = v8::String::new(scope, name)
                .ok_or_else(|| anyhow::anyhow!("V8 string alloc failed"))?;
            let arg1 = v8::String::new(scope, params_json)
                .ok_or_else(|| anyhow::anyhow!("V8 string alloc failed"))?;

            let result = v8_call!(scope, func, undef.into(), &[arg0.into(), arg1.into()])
                .map_err(|e| anyhow::anyhow!("MCP tool error: {e}"))?;

            result.to_rust_string_lossy(scope)
        };

        if result_str == "__REX_MCP_ASYNC__" {
            self.isolate.perform_microtask_checkpoint();

            v8::scope_with_context!(scope, &mut self.isolate, &self.context);
            let resolve_result = v8_eval!(scope, "globalThis.__rex_resolve_mcp()", "<mcp-resolve>")
                .map_err(|e| anyhow::anyhow!("MCP tool error: {e}"))?;
            Ok(resolve_result.to_rust_string_lossy(scope))
        } else {
            Ok(result_str)
        }
    }
}

/// Replace lone surrogate JSON hex escapes (`\uD800`–`\uDFFF`) with `\uFFFD`.
/// React's flight protocol can emit unpaired surrogates that are valid in JS
/// strings but rejected by serde_json (strict RFC 8259).
fn sanitize_json_surrogates(s: &str) -> String {
    let bytes = s.as_bytes();
    let len = bytes.len();
    let mut out = String::with_capacity(len);
    let mut i = 0;
    while i < len {
        if bytes[i] == b'\\' && i + 5 < len && bytes[i + 1] == b'u' {
            if let Ok(hex) =
                u16::from_str_radix(std::str::from_utf8(&bytes[i + 2..i + 6]).unwrap_or(""), 16)
            {
                if (0xD800..=0xDBFF).contains(&hex) {
                    // High surrogate — check if followed by a low surrogate
                    if i + 11 < len
                        && bytes[i + 6] == b'\\'
                        && bytes[i + 7] == b'u'
                        && u16::from_str_radix(
                            std::str::from_utf8(&bytes[i + 8..i + 12]).unwrap_or(""),
                            16,
                        )
                        .is_ok_and(|low| (0xDC00..=0xDFFF).contains(&low))
                    {
                        // Valid surrogate pair — keep both
                        out.push_str(&s[i..i + 12]);
                        i += 12;
                        continue;
                    }
                    // Lone high surrogate — replace
                    out.push_str("\\uFFFD");
                    i += 6;
                    continue;
                } else if (0xDC00..=0xDFFF).contains(&hex) {
                    // Lone low surrogate — replace
                    out.push_str("\\uFFFD");
                    i += 6;
                    continue;
                }
            }
        }
        // Advance by one UTF-8 character (the string is known-valid UTF-8).
        if let Some(ch) = s[i..].chars().next() {
            out.push(ch);
            i += ch.len_utf8();
        } else {
            break;
        }
    }
    out
}

impl SsrIsolate {
    /// Set request context (headers/cookies) in V8 globals before action execution.
    pub fn set_request_context(&mut self, headers_json: &str, cookies_json: &str) -> Result<()> {
        v8::scope_with_context!(scope, &mut self.isolate, &self.context);
        let code = format!(
            "globalThis.__rex_request_headers = {}; globalThis.__rex_request_cookies = {};",
            headers_json, cookies_json
        );
        v8_eval!(scope, &code, "<set-request-context>")?;
        Ok(())
    }

    /// Clear request context after action execution.
    pub fn clear_request_context(&mut self) -> Result<()> {
        v8::scope_with_context!(scope, &mut self.isolate, &self.context);
        v8_eval!(
            scope,
            "globalThis.__rex_request_headers = {}; globalThis.__rex_request_cookies = {};",
            "<clear-request-context>"
        )?;
        Ok(())
    }

    /// Call __rex_call_server_action(actionId, argsJson) and return JSON response.
    /// Handles async actions by pumping V8's microtask queue + fetch loop.
    pub fn call_server_action(&mut self, action_id: &str, args_json: &str) -> Result<String> {
        let action_fn = self
            .server_action_fn
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Server actions not loaded"))?;

        let result_str = {
            v8::scope_with_context!(scope, &mut self.isolate, &self.context);

            let func = v8::Local::new(scope, action_fn);
            let undef = v8::undefined(scope);
            let arg0 = v8::String::new(scope, action_id)
                .ok_or_else(|| anyhow::anyhow!("V8 string alloc failed"))?;
            let arg1 = v8::String::new(scope, args_json)
                .ok_or_else(|| anyhow::anyhow!("V8 string alloc failed"))?;

            let result = v8_call!(scope, func, undef.into(), &[arg0.into(), arg1.into()])
                .map_err(|e| anyhow::anyhow!("Server action error: {e}"))?;

            result.to_rust_string_lossy(scope)
        };

        self.pump_action_loop(&result_str)
    }

    /// Call __rex_call_server_action_encoded(actionId, body, isFormFields) using React's decodeReply.
    /// The body is an encoded string from the client's encodeReply.
    /// When `is_form_fields` is true, body is JSON-encoded form fields (multipart).
    /// Always async since decodeReply returns a Promise.
    pub fn call_server_action_encoded(
        &mut self,
        action_id: &str,
        body: &str,
        is_form_fields: bool,
    ) -> Result<String> {
        let action_fn = self
            .server_action_encoded_fn
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Encoded server actions not loaded"))?;

        let result_str = {
            v8::scope_with_context!(scope, &mut self.isolate, &self.context);

            let func = v8::Local::new(scope, action_fn);
            let undef = v8::undefined(scope);
            let arg0 = v8::String::new(scope, action_id)
                .ok_or_else(|| anyhow::anyhow!("V8 string alloc failed"))?;
            let arg1 = v8::String::new(scope, body)
                .ok_or_else(|| anyhow::anyhow!("V8 string alloc failed"))?;
            let arg2 = v8::Boolean::new(scope, is_form_fields);

            let result = v8_call!(
                scope,
                func,
                undef.into(),
                &[arg0.into(), arg1.into(), arg2.into()]
            )
            .map_err(|e| anyhow::anyhow!("Encoded server action error: {e}"))?;

            result.to_rust_string_lossy(scope)
        };

        self.pump_action_loop(&result_str)
    }

    /// Call __rex_call_form_action(fieldsJson) using React's decodeAction.
    /// fieldsJson is a JSON array of [key, value] pairs from multipart parsing.
    /// The action ID is extracted from the FormData by React's decodeAction.
    pub fn call_form_action(&mut self, _action_id: &str, fields_json: &str) -> Result<String> {
        let action_fn = self
            .form_action_fn
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Form actions not loaded"))?;

        let result_str = {
            v8::scope_with_context!(scope, &mut self.isolate, &self.context);

            let func = v8::Local::new(scope, action_fn);
            let undef = v8::undefined(scope);
            let arg0 = v8::String::new(scope, fields_json)
                .ok_or_else(|| anyhow::anyhow!("V8 string alloc failed"))?;

            let result = v8_call!(scope, func, undef.into(), &[arg0.into()])
                .map_err(|e| anyhow::anyhow!("Form action error: {e}"))?;

            result.to_rust_string_lossy(scope)
        };

        self.pump_action_loop(&result_str)
    }

    /// Shared async resolution loop for server action results.
    /// Pumps V8 microtasks and the fetch loop until the action resolves.
    fn pump_action_loop(&mut self, initial_result: &str) -> Result<String> {
        if initial_result == "__REX_ACTION_ASYNC__" {
            crate::fetch::run_fetch_loop(&mut self.isolate, &self.context);

            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
            loop {
                if std::time::Instant::now() > deadline {
                    return Err(anyhow::anyhow!("Server action timed out after 30s"));
                }

                crate::fetch::run_fetch_loop(&mut self.isolate, &self.context);

                let status = {
                    v8::scope_with_context!(scope, &mut self.isolate, &self.context);
                    let result = v8_eval!(
                        scope,
                        "globalThis.__rex_resolve_action_pending()",
                        "<action-resolve>"
                    )
                    .map_err(|e| anyhow::anyhow!("Server action resolve error: {e}"))?;
                    result.to_rust_string_lossy(scope)
                };

                match status.as_str() {
                    "done" => break,
                    "pending" => {
                        std::thread::sleep(std::time::Duration::from_millis(1));
                        continue;
                    }
                    other => {
                        return Err(anyhow::anyhow!(
                            "Unexpected action resolve status: {}",
                            other
                        ));
                    }
                }
            }

            v8::scope_with_context!(scope, &mut self.isolate, &self.context);
            let resolve_result = v8_eval!(
                scope,
                "globalThis.__rex_finalize_action()",
                "<action-finalize>"
            )
            .map_err(|e| anyhow::anyhow!("Server action finalize error: {e}"))?;
            Ok(resolve_result.to_rust_string_lossy(scope))
        } else {
            Ok(initial_result.to_string())
        }
    }
}
