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
    overlay.style.cssText = [
      'position: fixed',
      'top: 0',
      'left: 0',
      'width: 100%',
      'height: 100%',
      'background: rgba(0, 0, 0, 0.85)',
      'color: #ff5555',
      'font-family: monospace',
      'font-size: 14px',
      'padding: 40px',
      'z-index: 99999',
      'overflow: auto',
      'box-sizing: border-box',
    ].join(';');

    var title = document.createElement('h2');
    title.style.cssText = 'color: #ff5555; margin: 0 0 20px 0; font-size: 20px;';
    title.textContent = 'Build Error';

    var fileEl = null;
    if (file) {
      fileEl = document.createElement('div');
      fileEl.style.cssText = 'color: #888; margin-bottom: 10px;';
      fileEl.textContent = file;
    }

    var pre = document.createElement('pre');
    pre.style.cssText = 'white-space: pre-wrap; word-wrap: break-word; color: #fff;';
    pre.textContent = message;

    var dismiss = document.createElement('button');
    dismiss.textContent = 'Dismiss';
    dismiss.style.cssText = [
      'position: absolute',
      'top: 10px',
      'right: 10px',
      'background: none',
      'border: 1px solid #666',
      'color: #999',
      'padding: 5px 10px',
      'cursor: pointer',
      'font-family: monospace',
    ].join(';');
    dismiss.onclick = removeOverlay;

    overlay.appendChild(dismiss);
    overlay.appendChild(title);
    if (fileEl) overlay.appendChild(fileEl);
    overlay.appendChild(pre);

    document.body.appendChild(overlay);
  }

  function removeOverlay() {
    if (overlay && overlay.parentNode) {
      overlay.parentNode.removeChild(overlay);
    }
    overlay = null;
  }

  connect();
})();
