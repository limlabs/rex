// Rex HMR Client
(function() {
  'use strict';

  var protocol = location.protocol === 'https:' ? 'wss:' : 'ws:';
  var url = protocol + '//' + location.host + '/_rex/hmr';
  var ws;
  var overlay = null;
  var tscOverlay = null;

  function connect() {
    ws = new WebSocket(url);

    ws.onopen = function() {
      console.log('[Rex HMR] Connected');
      removeOverlay();
    };

    ws.onmessage = function(event) {
      var msg;
      try {
        msg = JSON.parse(event.data);
      } catch {
        return;
      }

      switch (msg.type) {
        case 'connected':
          console.log('[Rex HMR] Ready');
          break;

        case 'update':
          console.log('[Rex HMR] Update:', msg.path);
          removeOverlay();
          hotUpdate(msg);
          break;

        case 'full-reload':
          console.log('[Rex HMR] Full reload');
          window.location.reload();
          break;

        case 'error':
          console.error('[Rex HMR] Error:', msg.message);
          showOverlay(msg.message, msg.file);
          break;

        case 'tsc-error':
          console.error('[Rex HMR] Type errors:', msg.errors.length);
          showTscOverlay(msg.errors);
          break;

        case 'tsc-clear':
          console.log('[Rex HMR] Type errors resolved');
          removeTscOverlay();
          break;
      }
    };

    ws.onclose = function() {
      console.log('[Rex HMR] Disconnected, reconnecting in 1s...');
      setTimeout(connect, 1000);
    };

    ws.onerror = function() {
      ws.close();
    };
  }

  function showOverlay(message, file) {
    removeOverlay();

    overlay = document.createElement('div');
    overlay.id = '__rex_error_overlay';
    overlay.innerHTML = '<style>' +
      '#__rex_error_overlay{position:fixed;top:0;left:0;width:100%;height:100%;' +
      'background:#1a1a2e;color:#e0e0e0;font-family:"SF Mono","Fira Code","JetBrains Mono",Menlo,Consolas,monospace;' +
      'font-size:14px;z-index:99999;overflow:auto;box-sizing:border-box;display:flex;align-items:flex-start;justify-content:center;padding:60px 20px}' +
      '#__rex_error_overlay .eo-container{max-width:860px;width:100%}' +
      '#__rex_error_overlay .eo-badge{display:inline-block;background:#e63946;color:#fff;font-size:11px;font-weight:700;' +
      'text-transform:uppercase;letter-spacing:.5px;padding:4px 10px;border-radius:4px;margin-bottom:16px}' +
      '#__rex_error_overlay .eo-file{color:#8892b0;font-size:13px;margin-bottom:16px;padding:8px 12px;' +
      'background:rgba(255,255,255,.04);border-radius:6px;border-left:3px solid #e63946}' +
      '#__rex_error_overlay .eo-stack{background:#0d1117;border:1px solid rgba(255,255,255,.08);border-radius:8px;' +
      'padding:20px;overflow-x:auto;font-size:13px;line-height:1.7;white-space:pre-wrap;word-wrap:break-word;color:#f0c674}' +
      '#__rex_error_overlay .eo-hint{margin-top:24px;font-size:12px;color:#555}' +
      '#__rex_error_overlay .eo-dot{display:inline-block;width:8px;height:8px;border-radius:50%;margin-right:8px;background:#2ecc71}' +
      '#__rex_error_overlay .eo-dismiss{position:absolute;top:20px;right:20px;background:none;border:1px solid rgba(255,255,255,.15);' +
      'color:#888;padding:6px 14px;cursor:pointer;font-family:inherit;font-size:12px;border-radius:4px}' +
      '#__rex_error_overlay .eo-dismiss:hover{border-color:rgba(255,255,255,.3);color:#bbb}' +
      '</style>' +
      '<div class="eo-container">' +
      '<button class="eo-dismiss" id="__rex_eo_dismiss">Dismiss</button>' +
      '<div class="eo-badge">Build Error</div>' +
      (file ? '<div class="eo-file" id="__rex_eo_file"></div>' : '') +
      '<div class="eo-stack" id="__rex_eo_msg"></div>' +
      '<div class="eo-hint"><span class="eo-dot"></span>Connected — save a file to reload</div>' +
      '</div>';

    document.body.appendChild(overlay);

    // Set text content (safe from XSS)
    document.getElementById('__rex_eo_msg').textContent = message;
    if (file && document.getElementById('__rex_eo_file')) {
      document.getElementById('__rex_eo_file').textContent = file;
    }
    document.getElementById('__rex_eo_dismiss').onclick = removeOverlay;
  }

  function removeOverlay() {
    if (overlay && overlay.parentNode) {
      overlay.parentNode.removeChild(overlay);
    }
    overlay = null;
  }

  function showTscOverlay(errors) {
    removeTscOverlay();

    tscOverlay = document.createElement('div');
    tscOverlay.id = '__rex_tsc_overlay';

    var lines = '';
    for (var i = 0; i < errors.length; i++) {
      lines += '<div class="tsc-diag">' +
        '<span class="tsc-loc"></span>' +
        '<span class="tsc-code"></span>' +
        '<span class="tsc-msg"></span>' +
        '</div>';
    }

    tscOverlay.innerHTML = '<style>' +
      '#__rex_tsc_overlay{position:fixed;top:0;left:0;width:100%;height:100%;' +
      'background:#1a1a2e;color:#e0e0e0;font-family:"SF Mono","Fira Code","JetBrains Mono",Menlo,Consolas,monospace;' +
      'font-size:14px;z-index:99998;overflow:auto;box-sizing:border-box;display:flex;align-items:flex-start;justify-content:center;padding:60px 20px}' +
      '#__rex_tsc_overlay .eo-container{max-width:860px;width:100%}' +
      '#__rex_tsc_overlay .eo-badge{display:inline-block;background:#e67e22;color:#fff;font-size:11px;font-weight:700;' +
      'text-transform:uppercase;letter-spacing:.5px;padding:4px 10px;border-radius:4px;margin-bottom:16px}' +
      '#__rex_tsc_overlay .eo-count{display:inline-block;color:#8892b0;font-size:12px;margin-left:10px}' +
      '#__rex_tsc_overlay .tsc-list{background:#0d1117;border:1px solid rgba(255,255,255,.08);border-radius:8px;' +
      'padding:16px 20px;overflow-x:auto;font-size:13px;line-height:1.8}' +
      '#__rex_tsc_overlay .tsc-diag{border-bottom:1px solid rgba(255,255,255,.05);padding:6px 0}' +
      '#__rex_tsc_overlay .tsc-diag:last-child{border-bottom:none}' +
      '#__rex_tsc_overlay .tsc-loc{color:#8892b0}' +
      '#__rex_tsc_overlay .tsc-code{color:#e63946;margin:0 8px}' +
      '#__rex_tsc_overlay .tsc-msg{color:#f0c674}' +
      '#__rex_tsc_overlay .eo-hint{margin-top:24px;font-size:12px;color:#555}' +
      '#__rex_tsc_overlay .eo-dot{display:inline-block;width:8px;height:8px;border-radius:50%;margin-right:8px;background:#2ecc71}' +
      '#__rex_tsc_overlay .eo-dismiss{position:absolute;top:20px;right:20px;background:none;border:1px solid rgba(255,255,255,.15);' +
      'color:#888;padding:6px 14px;cursor:pointer;font-family:inherit;font-size:12px;border-radius:4px}' +
      '#__rex_tsc_overlay .eo-dismiss:hover{border-color:rgba(255,255,255,.3);color:#bbb}' +
      '</style>' +
      '<div class="eo-container">' +
      '<button class="eo-dismiss" id="__rex_tsc_dismiss">Dismiss</button>' +
      '<div class="eo-badge">Type Error</div>' +
      '<span class="eo-count">' + errors.length + (errors.length === 1 ? ' error' : ' errors') + '</span>' +
      '<div class="tsc-list">' + lines + '</div>' +
      '<div class="eo-hint"><span class="eo-dot"></span>Connected — fix type errors and save</div>' +
      '</div>';

    document.body.appendChild(tscOverlay);

    // Set text content safely per diagnostic
    var diags = tscOverlay.querySelectorAll('.tsc-diag');
    for (var i = 0; i < errors.length; i++) {
      var e = errors[i];
      diags[i].querySelector('.tsc-loc').textContent = e.file + '(' + e.line + ',' + e.col + ')';
      diags[i].querySelector('.tsc-code').textContent = e.code;
      diags[i].querySelector('.tsc-msg').textContent = e.message;
    }

    document.getElementById('__rex_tsc_dismiss').onclick = removeTscOverlay;
  }

  function removeTscOverlay() {
    if (tscOverlay && tscOverlay.parentNode) {
      tscOverlay.parentNode.removeChild(tscOverlay);
    }
    tscOverlay = null;
  }

  // --- Hot update: re-import changed page module and re-render in place ---

  function hotUpdate(msg) {
    var manifest = window.__REX_MANIFEST__;
    var newManifest = msg.manifest;

    if (!manifest || !newManifest || !newManifest.pages) {
      console.log('[Rex HMR] No manifest, falling back to full reload');
      window.location.reload();
      return;
    }

    // Figure out which route pattern we're currently on
    var router = window.__REX_ROUTER;
    var currentPattern = router && router.state ? router.state.route : null;

    if (!currentPattern || !newManifest.pages[currentPattern]) {
      // Current page isn't in the new manifest (removed?), full reload
      console.log('[Rex HMR] Current route not in manifest, falling back to full reload');
      window.location.reload();
      return;
    }

    // Update manifest in place so the router's closure references stay valid
    manifest.build_id = newManifest.build_id;
    for (var pattern in newManifest.pages) {
      manifest.pages[pattern] = newManifest.pages[pattern];
    }

    // Clear old page module so ensureChunk will re-import
    if (window.__REX_PAGES) {
      delete window.__REX_PAGES[currentPattern];
    }

    var newChunk = newManifest.pages[currentPattern].js;
    var chunkUrl = '/_rex/static/' + newChunk;

    // Dynamic import with cache-bust (chunk filename already has new hash)
    window.__REX_NAVIGATING__ = true;
    import(chunkUrl).then(function() {
      window.__REX_NAVIGATING__ = false;

      // Fetch fresh GSSP data
      var dataUrl = '/_rex/data/' + newManifest.build_id + window.location.pathname + '.json';
      return fetch(dataUrl).then(function(res) {
        if (!res.ok) throw new Error('Data fetch failed: ' + res.status);
        return res.json();
      });
    }).then(function(data) {
      var props = data.props || {};

      // Update the data element
      var dataEl = document.getElementById('__REX_DATA__');
      if (dataEl) dataEl.textContent = JSON.stringify(props);

      // Re-render with the new page component
      var page = window.__REX_PAGES && window.__REX_PAGES[currentPattern];
      if (page && window.__REX_RENDER__) {
        window.__REX_RENDER__(page.default, props);
        console.log('[Rex HMR] Hot update applied');
      } else {
        console.log('[Rex HMR] Could not re-render, falling back to full reload');
        window.location.reload();
      }
    }).catch(function(err) {
      console.error('[Rex HMR] Hot update failed, falling back to full reload:', err);
      window.location.reload();
    });
  }

  connect();
})();
