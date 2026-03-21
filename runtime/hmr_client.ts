// Rex HMR Client
(function () {
  "use strict";

  const protocol = location.protocol === "https:" ? "wss:" : "ws:";
  const url = protocol + "//" + location.host + "/_rex/hmr";
  let ws: WebSocket;
  let overlay: HTMLDivElement | null = null;
  let minimized = false;
  let badge: HTMLDivElement | null = null;
  let currentError: {
    message: string;
    file?: string;
    kind?: string;
  } | null = null;
  let tscErrors: RexTscDiagnostic[] = [];
  let activeEscListener: ((e: KeyboardEvent) => void) | null = null;
  let pendingReload = false;

  function connect(): void {
    ws = new WebSocket(url);

    ws.onopen = function () {
      console.log("[Rex HMR] Connected");
      clearAllErrors();
    };

    ws.onmessage = function (event: MessageEvent) {
      let msg: RexHmrMessage;
      try {
        msg = JSON.parse(event.data as string) as RexHmrMessage;
      } catch {
        return;
      }

      switch (msg.type) {
        case "connected":
          console.log("[Rex HMR] Ready");
          break;

        case "update":
          console.log("[Rex HMR] Update:", msg.path);
          clearAllErrors();
          hotUpdate(msg);
          break;

        case "full-reload":
          scheduleReload();
          break;

        case "error":
          console.error("[Rex HMR] Error:", msg.message);
          currentError = {
            message: msg.message || "Unknown error",
            file: msg.file,
            kind: msg.kind || "build",
          };
          tscErrors = [];
          showOverlay();
          break;

        case "tsc-error":
          console.error("[Rex HMR] TypeScript errors:", msg.errors?.length);
          tscErrors = msg.errors || [];
          if (currentError?.kind === "typescript") {
            currentError.message = formatTscErrors(tscErrors);
          } else if (!currentError) {
            currentError = {
              message: formatTscErrors(tscErrors),
              kind: "typescript",
            };
          }
          showOverlay();
          break;

        case "tsc-clear":
          tscErrors = [];
          if (currentError?.kind === "typescript") {
            clearAllErrors();
          }
          break;
      }
    };

    ws.onclose = function () {
      console.log("[Rex HMR] Disconnected, reconnecting in 1s...");
      setTimeout(connect, 1000);
    };

    ws.onerror = function () {
      ws.close();
    };
  }

  // --- Client-side error capture ---

  window.addEventListener("error", function (event: ErrorEvent) {
    const stack = event.error?.stack || event.message;
    currentError = {
      message: stack,
      file: event.filename
        ? event.filename + ":" + event.lineno + ":" + event.colno
        : undefined,
      kind: "client",
    };
    showOverlay();
  });

  window.addEventListener(
    "unhandledrejection",
    function (event: PromiseRejectionEvent) {
      const reason = event.reason;
      const msg =
        reason instanceof Error
          ? reason.stack || reason.message
          : String(reason);
      currentError = {
        message: msg,
        kind: "client",
      };
      showOverlay();
    },
  );

  // --- Error formatting ---

  function formatTscErrors(errors: RexTscDiagnostic[]): string {
    return errors
      .map(
        (e) =>
          e.file +
          "(" +
          e.line +
          "," +
          e.col +
          "): error " +
          e.code +
          ": " +
          e.message,
      )
      .join("\n");
  }

  function kindLabel(kind: string): string {
    switch (kind) {
      case "build":
        return "Build Error";
      case "server":
        return "Server Error";
      case "client":
        return "Runtime Error";
      case "typescript":
        return "Type Error";
      default:
        return "Error";
    }
  }

  function kindBadgeClass(kind: string): string {
    switch (kind) {
      case "server":
        return "eo-badge eo-badge-server";
      case "client":
        return "eo-badge eo-badge-client";
      case "typescript":
        return "eo-badge eo-badge-ts";
      default:
        return "eo-badge";
    }
  }

  function originLabel(kind: string): string {
    switch (kind) {
      case "build":
      case "server":
      case "typescript":
        return "Server";
      case "client":
        return "Client";
      default:
        return "";
    }
  }

  // --- Source map integration for stack traces ---

  // Parse a stack trace string and attempt to map through source maps
  // For client errors, the browser has already applied source maps to Error.stack
  // For server errors, we display as-is (server should map before sending)
  function formatStack(message: string, kind: string): string {
    if (kind === "client") {
      // Browser already applies source maps to Error.stack
      // Just clean up the trace for display
      return cleanStack(message);
    }
    // Build/server errors: display as received
    return message;
  }

  function cleanStack(stack: string): string {
    // Remove noise lines (internal framework, node_modules chunks)
    return stack
      .split("\n")
      .filter((line) => {
        const trimmed = line.trim();
        // Keep error message lines (don't start with "at ")
        if (!trimmed.startsWith("at ")) return true;
        // Filter out internal rex runtime frames
        if (trimmed.includes("/_rex/")) return false;
        // Filter out node_modules internals
        if (trimmed.includes("/node_modules/")) return false;
        return true;
      })
      .join("\n");
  }

  // --- Overlay UI ---

  function showOverlay(): void {
    if (!currentError) return;

    // If minimized, just show the badge
    if (minimized) {
      removeBadge();
      showBadge();
      removeOverlayEl();
      return;
    }

    removeOverlayEl();
    removeBadge();

    const err = currentError;
    // Sanitize kind — switch assigns only literal strings (breaks taint chain for CodeQL)
    let kind: string;
    switch (err.kind) {
      case "server":
        kind = "server";
        break;
      case "client":
        kind = "client";
        break;
      case "typescript":
        kind = "typescript";
        break;
      default:
        kind = "build";
        break;
    }
    const origin = originLabel(kind);
    const formattedStack = formatStack(err.message, kind);

    overlay = document.createElement("div");
    overlay.id = "__rex_error_overlay";
    overlay.innerHTML =
      "<style>" + overlayStyles() + "</style>" +
      '<div class="eo-backdrop" id="__rex_eo_backdrop"></div>' +
      '<div class="eo-dialog">' +
      '<div class="eo-header">' +
      '<div class="eo-header-left">' +
      (origin
        ? '<span class="eo-origin eo-origin-' + kind + '">' + origin + "</span>"
        : "") +
      '<span class="' + kindBadgeClass(kind) + '">' + kindLabel(kind) + "</span>" +
      "</div>" +
      '<div class="eo-header-right">' +
      '<button class="eo-btn eo-minimize" id="__rex_eo_min" title="Minimize">−</button>' +
      '<button class="eo-btn eo-dismiss" id="__rex_eo_dismiss" title="Dismiss">×</button>' +
      "</div>" +
      "</div>" +
      (err.file ? '<div class="eo-file" id="__rex_eo_file"></div>' : "") +
      '<pre class="eo-stack" id="__rex_eo_msg"></pre>' +
      (tscErrors.length > 0 && kind !== "typescript"
        ? '<div class="eo-tsc-section">' +
          '<div class="eo-tsc-header">TypeScript Errors (' + tscErrors.length + ")</div>" +
          '<pre class="eo-stack eo-tsc-stack" id="__rex_eo_tsc"></pre>' +
          "</div>"
        : "") +
      '<div class="eo-hint"><span class="eo-dot"></span>Connected — save a file to reload</div>' +
      "</div>";

    document.body.appendChild(overlay);

    // Set text content safely (XSS-safe)
    const msgEl = document.getElementById("__rex_eo_msg");
    if (msgEl) msgEl.textContent = formattedStack;
    if (err.file) {
      const fileEl = document.getElementById("__rex_eo_file");
      if (fileEl) fileEl.textContent = err.file;
    }
    if (tscErrors.length > 0 && kind !== "typescript") {
      const tscEl = document.getElementById("__rex_eo_tsc");
      if (tscEl) tscEl.textContent = formatTscErrors(tscErrors);
    }

    document.getElementById("__rex_eo_dismiss")!.onclick = function () {
      removeEscListener();
      clearAllErrors();
    };
    document.getElementById("__rex_eo_min")!.onclick = function () {
      removeEscListener();
      minimized = true;
      showOverlay();
    };
    document.getElementById("__rex_eo_backdrop")!.onclick = function () {
      removeEscListener();
      minimized = true;
      showOverlay();
    };

    // ESC to minimize — listen on document since div elements can't receive focus
    removeEscListener();
    activeEscListener = function (e: KeyboardEvent): void {
      if (e.key === "Escape") {
        removeEscListener();
        minimized = true;
        showOverlay();
      }
    };
    document.addEventListener("keydown", activeEscListener);
  }

  function showBadge(): void {
    if (!currentError) return;

    badge = document.createElement("div");
    badge.id = "__rex_error_badge";
    const kind = currentError.kind || "build";
    badge.innerHTML =
      "<style>" +
      "#__rex_error_badge{position:fixed;bottom:16px;right:16px;z-index:99999;" +
      "cursor:pointer;display:flex;align-items:center;gap:8px;" +
      "background:#1a1a2e;border:1px solid #e63946;border-radius:8px;" +
      "padding:8px 14px;font-family:'SF Mono','Fira Code','JetBrains Mono',Menlo,Consolas,monospace;" +
      "font-size:12px;color:#e0e0e0;box-shadow:0 4px 12px rgba(0,0,0,0.4);transition:opacity .15s}" +
      "#__rex_error_badge:hover{opacity:0.9}" +
      "#__rex_error_badge .badge-dot{width:8px;height:8px;border-radius:50%;background:#e63946;" +
      "animation:rex-pulse 2s infinite}" +
      "@keyframes rex-pulse{0%,100%{opacity:1}50%{opacity:0.3}}" +
      "</style>" +
      '<span class="badge-dot"></span>' +
      "<span>" + kindLabel(kind) + "</span>";

    document.body.appendChild(badge);

    badge.onclick = function () {
      minimized = false;
      showOverlay();
    };
  }

  function removeEscListener(): void {
    if (activeEscListener) {
      document.removeEventListener("keydown", activeEscListener);
      activeEscListener = null;
    }
  }

  function clearAllErrors(): void {
    currentError = null;
    tscErrors = [];
    minimized = false;
    removeEscListener();
    removeOverlayEl();
    removeBadge();
  }

  function removeOverlayEl(): void {
    if (overlay && overlay.parentNode) {
      overlay.parentNode.removeChild(overlay);
    }
    overlay = null;
  }

  function removeBadge(): void {
    if (badge && badge.parentNode) {
      badge.parentNode.removeChild(badge);
    }
    badge = null;
  }

  function overlayStyles(): string {
    return (
      "#__rex_error_overlay{position:fixed;top:0;left:0;width:100%;height:100%;" +
      "z-index:99999;display:flex;align-items:flex-start;justify-content:center}" +
      "#__rex_error_overlay .eo-backdrop{position:absolute;top:0;left:0;width:100%;height:100%;" +
      "background:rgba(0,0,0,0.6);backdrop-filter:blur(2px)}" +
      "#__rex_error_overlay .eo-dialog{position:relative;max-width:860px;width:calc(100% - 40px);" +
      "margin-top:60px;background:#1a1a2e;border:1px solid rgba(255,255,255,0.1);" +
      "border-radius:12px;padding:24px;box-shadow:0 8px 32px rgba(0,0,0,0.5);" +
      'font-family:"SF Mono","Fira Code","JetBrains Mono",Menlo,Consolas,monospace;' +
      "font-size:14px;color:#e0e0e0;max-height:calc(100vh - 120px);overflow-y:auto}" +
      // Header
      "#__rex_error_overlay .eo-header{display:flex;align-items:center;justify-content:space-between;margin-bottom:16px}" +
      "#__rex_error_overlay .eo-header-left{display:flex;align-items:center;gap:8px}" +
      "#__rex_error_overlay .eo-header-right{display:flex;gap:4px}" +
      // Origin badge (Server / Client)
      "#__rex_error_overlay .eo-origin{font-size:11px;font-weight:600;letter-spacing:.5px;" +
      "padding:3px 8px;border-radius:4px;text-transform:uppercase}" +
      "#__rex_error_overlay .eo-origin-build,#__rex_error_overlay .eo-origin-server," +
      "#__rex_error_overlay .eo-origin-typescript{background:rgba(99,102,241,0.15);color:#818cf8}" +
      "#__rex_error_overlay .eo-origin-client{background:rgba(251,191,36,0.15);color:#fbbf24}" +
      // Error type badge
      "#__rex_error_overlay .eo-badge{display:inline-block;background:#e63946;color:#fff;font-size:11px;font-weight:700;" +
      "text-transform:uppercase;letter-spacing:.5px;padding:3px 8px;border-radius:4px}" +
      "#__rex_error_overlay .eo-badge-server{background:#7c3aed}" +
      "#__rex_error_overlay .eo-badge-client{background:#d97706}" +
      "#__rex_error_overlay .eo-badge-ts{background:#3178c6}" +
      // Buttons
      "#__rex_error_overlay .eo-btn{background:none;border:1px solid rgba(255,255,255,0.15);" +
      "color:#888;width:28px;height:28px;cursor:pointer;font-family:inherit;font-size:16px;" +
      "border-radius:6px;display:flex;align-items:center;justify-content:center;line-height:1}" +
      "#__rex_error_overlay .eo-btn:hover{border-color:rgba(255,255,255,0.3);color:#bbb}" +
      // File path
      "#__rex_error_overlay .eo-file{color:#8892b0;font-size:13px;margin-bottom:16px;padding:8px 12px;" +
      "background:rgba(255,255,255,0.04);border-radius:6px;border-left:3px solid #e63946}" +
      // Stack trace
      "#__rex_error_overlay .eo-stack{background:#0d1117;border:1px solid rgba(255,255,255,0.08);border-radius:8px;" +
      "padding:20px;overflow-x:auto;font-size:13px;line-height:1.7;white-space:pre-wrap;word-wrap:break-word;" +
      "color:#f0c674;margin:0}" +
      // TSC section
      "#__rex_error_overlay .eo-tsc-section{margin-top:16px}" +
      "#__rex_error_overlay .eo-tsc-header{font-size:12px;font-weight:600;color:#3178c6;margin-bottom:8px;" +
      "text-transform:uppercase;letter-spacing:.5px}" +
      "#__rex_error_overlay .eo-tsc-stack{border-left:3px solid #3178c6}" +
      // Hint
      "#__rex_error_overlay .eo-hint{margin-top:16px;font-size:12px;color:#555;display:flex;align-items:center}" +
      "#__rex_error_overlay .eo-dot{display:inline-block;width:8px;height:8px;border-radius:50%;" +
      "margin-right:8px;background:#2ecc71}"
    );
  }

  // --- Hot update: re-import changed page module and re-render in place ---

  function hotUpdate(msg: RexHmrMessage): void {
    const manifest = window.__REX_MANIFEST__;
    const newManifest = msg.manifest;

    if (!manifest || !newManifest) {
      console.log("[Rex HMR] No manifest, falling back to full reload");
      window.location.reload();
      return;
    }

    // App Router pages use RSC Flight protocol — full reload for now
    if (
      newManifest.app_routes &&
      Object.keys(newManifest.app_routes).length > 0 &&
      !newManifest.pages
    ) {
      scheduleReload();
      return;
    }

    if (!newManifest.pages) {
      console.log(
        "[Rex HMR] No pages in manifest, falling back to full reload",
      );
      window.location.reload();
      return;
    }

    // Figure out which route pattern we're currently on
    const router = window.__REX_ROUTER;
    const currentPattern =
      router && router.state ? router.state.route : null;

    if (!currentPattern || !newManifest.pages[currentPattern]) {
      console.log(
        "[Rex HMR] Current route not in manifest, falling back to full reload",
      );
      window.location.reload();
      return;
    }

    // Update manifest in place so the router's closure references stay valid
    manifest.build_id = newManifest.build_id;
    for (const pattern in newManifest.pages) {
      manifest.pages[pattern] = newManifest.pages[pattern];
    }

    // Clear old page module so ensureChunk will re-import
    if (window.__REX_PAGES) {
      delete window.__REX_PAGES[currentPattern];
    }

    const newChunk = newManifest.pages[currentPattern].js;
    const chunkUrl = "/_rex/static/" + newChunk;

    // Dynamic import with cache-bust (chunk filename already has new hash)
    window.__REX_NAVIGATING__ = true;
    import(chunkUrl)
      .then(function () {
        window.__REX_NAVIGATING__ = false;

        // Fetch fresh GSSP data — validate build_id to prevent request forgery
        const buildId = newManifest.build_id;
        if (
          typeof buildId !== "string" ||
          !/^[a-zA-Z0-9_-]+$/.test(buildId)
        ) {
          throw new Error("Invalid build_id in manifest");
        }
        const dataUrl =
          "/_rex/data/" +
          buildId +
          window.location.pathname +
          ".json";
        return fetch(dataUrl).then(function (res) {
          if (!res.ok) throw new Error("Data fetch failed: " + res.status);
          return res.json() as Promise<{ props?: Record<string, unknown> }>;
        });
      })
      .then(function (data) {
        const props = (data.props || {}) as Record<string, unknown>;

        // Update the data element
        const dataEl = document.getElementById("__REX_DATA__");
        if (dataEl) dataEl.textContent = JSON.stringify(props);

        // Re-render with the new page component
        const page =
          window.__REX_PAGES && window.__REX_PAGES[currentPattern];
        if (page && window.__REX_RENDER__) {
          window.__REX_RENDER__(page.default, props);
          console.log("[Rex HMR] Hot update applied");
        } else {
          console.log(
            "[Rex HMR] Could not re-render, falling back to full reload",
          );
          window.location.reload();
        }
      })
      .catch(function (err) {
        console.error(
          "[Rex HMR] Hot update failed, falling back to full reload:",
          err,
        );
        window.location.reload();
      });
  }

  // Defer reload for background tabs — only reload when the tab becomes visible.
  // This prevents unnecessary reloads of unrelated pages in other tabs.
  function scheduleReload(): void {
    if (document.visibilityState === "visible") {
      console.log("[Rex HMR] Reloading");
      window.location.reload();
    } else {
      console.log("[Rex HMR] Tab hidden, deferring reload");
      pendingReload = true;
    }
  }

  document.addEventListener("visibilitychange", function () {
    if (document.visibilityState === "visible" && pendingReload) {
      pendingReload = false;
      console.log("[Rex HMR] Tab visible, applying deferred reload");
      window.location.reload();
    }
  });

  connect();
})();
