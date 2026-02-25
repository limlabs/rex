// Rex HMR Client
(function() {
  'use strict';

  var protocol = location.protocol === 'https:' ? 'wss:' : 'ws:';
  var url = protocol + '//' + location.host + '/_rex/hmr';
  var ws;
  var overlay = null;

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
      } catch (e) {
        return;
      }

      switch (msg.type) {
        case 'connected':
          console.log('[Rex HMR] Ready');
          break;

        case 'update':
          console.log('[Rex HMR] Update:', msg.path);
          removeOverlay();
          // For the prototype, do a full reload on any update
          // A full implementation would use React Fast Refresh here
          window.location.reload();
          break;

        case 'full-reload':
          console.log('[Rex HMR] Full reload');
          window.location.reload();
          break;

        case 'error':
          console.error('[Rex HMR] Error:', msg.message);
          showOverlay(msg.message, msg.file);
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

  connect();
})();
