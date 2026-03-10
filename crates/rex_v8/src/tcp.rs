//! TCP socket support for V8 isolates.
//!
//! Provides `globalThis.__rex_tcp_*` callbacks for the `net` module polyfill.
//!
//! Uses a **push-based** IO model (like Bun/Deno):
//! - `__rex_tcp_connect` / `__rex_tcp_write` / `__rex_tcp_close` are synchronous
//! - `__rex_tcp_enable_polling(connId)` registers a socket for push-based reads
//! - [`poll_tcp_sockets`] does non-blocking reads on all registered sockets
//!   and calls JS callbacks (`__rex_tcp_push_data/eof/error`) to push data directly
//! - No promises in the read path — eliminates idle connection deadlocks

use anyhow::Result;
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::io::{Read, Write};
use tracing::debug;

/// A TCP connection managed by the isolate thread.
enum TcpStream {
    Plain(std::net::TcpStream),
    Tls(Box<rustls::StreamOwned<rustls::ClientConnection, std::net::TcpStream>>),
}

impl TcpStream {
    fn set_nonblocking(&self, nonblocking: bool) -> std::io::Result<()> {
        match self {
            TcpStream::Plain(s) => s.set_nonblocking(nonblocking),
            TcpStream::Tls(s) => s.get_ref().set_nonblocking(nonblocking),
        }
    }
}

impl Read for TcpStream {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self {
            TcpStream::Plain(s) => s.read(buf),
            TcpStream::Tls(s) => s.read(buf),
        }
    }
}

impl Write for TcpStream {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        match self {
            TcpStream::Plain(s) => s.write(buf),
            TcpStream::Tls(s) => s.write(buf),
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        match self {
            TcpStream::Plain(s) => s.flush(),
            TcpStream::Tls(s) => s.flush(),
        }
    }
}

thread_local! {
    /// Active TCP connections on this isolate thread.
    static TCP_CONNECTIONS: RefCell<HashMap<u32, TcpStream>> = RefCell::new(HashMap::new());

    /// Set of connection IDs that are registered for push-based polling.
    /// A socket is added here after the JS `'connect'` event fires (parser is ready).
    static TCP_POLL_SET: RefCell<HashSet<u32>> = RefCell::new(HashSet::new());

    /// Next connection ID.
    static NEXT_TCP_ID: Cell<u32> = const { Cell::new(1) };

    /// Shared TLS client config (lazily initialized).
    static TLS_CONFIG: RefCell<Option<std::sync::Arc<rustls::ClientConfig>>> =
        const { RefCell::new(None) };
}

fn next_conn_id() -> u32 {
    NEXT_TCP_ID.with(|id| {
        let current = id.get();
        id.set(current.wrapping_add(1));
        current
    })
}

fn get_tls_config() -> std::sync::Arc<rustls::ClientConfig> {
    TLS_CONFIG.with(|cell| {
        let mut config = cell.borrow_mut();
        if let Some(ref cfg) = *config {
            return cfg.clone();
        }
        let root_store =
            rustls::RootCertStore::from_iter(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
        let cfg = std::sync::Arc::new(
            rustls::ClientConfig::builder()
                .with_root_certificates(root_store)
                .with_no_client_auth(),
        );
        *config = Some(cfg.clone());
        cfg
    })
}

/// Check whether any sockets are registered for polling.
pub fn has_active_tcp_sockets() -> bool {
    TCP_POLL_SET.with(|s| !s.borrow().is_empty())
}

// ── V8 callbacks ──────────────────────────────────────────────────────────

/// `__rex_tcp_connect(host, port)` → conn_id (u32)
pub fn tcp_connect_callback(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut ret: v8::ReturnValue,
) {
    if args.length() < 2 {
        let msg =
            v8::String::new(scope, "__rex_tcp_connect requires host and port").expect("v8 string");
        let err = v8::Exception::error(scope, msg);
        scope.throw_exception(err);
        return;
    }

    let host = args.get(0).to_rust_string_lossy(scope);
    let port = args.get(1).to_rust_string_lossy(scope);
    let addr = format!("{host}:{port}");

    match std::net::TcpStream::connect(&addr) {
        Ok(stream) => {
            let _ = stream.set_nodelay(true);

            let conn_id = next_conn_id();
            TCP_CONNECTIONS.with(|conns| {
                conns.borrow_mut().insert(conn_id, TcpStream::Plain(stream));
            });

            debug!(conn_id, addr = %addr, "TCP connection opened");
            ret.set(v8::Integer::new(scope, conn_id as i32).into());
        }
        Err(e) => {
            let msg = v8::String::new(scope, &format!("TCP connect failed for {addr}: {e}"))
                .expect("v8 string");
            let err = v8::Exception::error(scope, msg);
            scope.throw_exception(err);
        }
    }
}

/// `__rex_tcp_write(connId, data)` → bytes written (u32)
pub fn tcp_write_callback(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut ret: v8::ReturnValue,
) {
    if args.length() < 2 {
        let msg =
            v8::String::new(scope, "__rex_tcp_write requires connId and data").expect("v8 string");
        let err = v8::Exception::error(scope, msg);
        scope.throw_exception(err);
        return;
    }

    let conn_id = args.get(0).uint32_value(scope).unwrap_or(0);
    let data = args.get(1);

    let bytes: Vec<u8> = if let Ok(uint8) = v8::Local::<v8::Uint8Array>::try_from(data) {
        let len = uint8.byte_length();
        let mut buf = vec![0u8; len];
        uint8.copy_contents(&mut buf);
        buf
    } else if let Ok(ab) = v8::Local::<v8::ArrayBuffer>::try_from(data) {
        let len = ab.byte_length();
        let mut buf = vec![0u8; len];
        if len > 0 {
            let store = ab.get_backing_store();
            for (i, cell) in store.iter().enumerate().take(len) {
                buf[i] = cell.get();
            }
        }
        buf
    } else {
        data.to_rust_string_lossy(scope).into_bytes()
    };

    let result = TCP_CONNECTIONS.with(|conns| {
        let mut conns = conns.borrow_mut();
        let stream = conns
            .get_mut(&conn_id)
            .ok_or_else(|| format!("TCP connection {conn_id} not found"))?;
        stream
            .write_all(&bytes)
            .map_err(|e| format!("TCP write error: {e}"))?;
        stream
            .flush()
            .map_err(|e| format!("TCP flush error: {e}"))?;
        Ok::<usize, String>(bytes.len())
    });

    match result {
        Ok(n) => {
            ret.set(v8::Integer::new(scope, n as i32).into());
        }
        Err(e) => {
            debug!(conn_id, error = %e, "TCP write error");
            let msg = v8::String::new(scope, &e).expect("v8 string");
            let err = v8::Exception::error(scope, msg);
            scope.throw_exception(err);
        }
    }
}

/// `__rex_tcp_start_tls(connId, hostname)` → new conn_id (u32)
pub fn tcp_start_tls_callback(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut ret: v8::ReturnValue,
) {
    if args.length() < 2 {
        let msg = v8::String::new(scope, "__rex_tcp_start_tls requires connId and hostname")
            .expect("v8 string");
        let err = v8::Exception::error(scope, msg);
        scope.throw_exception(err);
        return;
    }

    let conn_id = args.get(0).uint32_value(scope).unwrap_or(0);
    let hostname = args.get(1).to_rust_string_lossy(scope);

    let result = TCP_CONNECTIONS.with(|conns| {
        let mut conns = conns.borrow_mut();
        let plain_stream = conns
            .remove(&conn_id)
            .ok_or_else(|| format!("TCP connection {conn_id} not found"))?;

        let tcp_stream = match plain_stream {
            TcpStream::Plain(s) => s,
            TcpStream::Tls(_) => {
                return Err("Connection is already TLS".to_string());
            }
        };

        let tls_config = get_tls_config();
        let server_name: rustls_pki_types::ServerName<'static> = hostname
            .clone()
            .try_into()
            .map_err(|e| format!("Invalid server name '{hostname}': {e}"))?;

        let tls_conn = rustls::ClientConnection::new(tls_config, server_name)
            .map_err(|e| format!("TLS handshake setup failed: {e}"))?;

        let tls_stream = rustls::StreamOwned::new(tls_conn, tcp_stream);
        let new_id = next_conn_id();
        conns.insert(new_id, TcpStream::Tls(Box::new(tls_stream)));

        debug!(old_id = conn_id, new_id, hostname = %hostname, "TLS upgrade complete");
        Ok(new_id)
    });

    // Also remove old conn from poll set (JS handles adding the new one)
    TCP_POLL_SET.with(|s| {
        s.borrow_mut().remove(&conn_id);
    });

    match result {
        Ok(new_id) => {
            ret.set(v8::Integer::new(scope, new_id as i32).into());
        }
        Err(e) => {
            let msg = v8::String::new(scope, &e).expect("v8 string");
            let err = v8::Exception::error(scope, msg);
            scope.throw_exception(err);
        }
    }
}

/// `__rex_tcp_enable_polling(connId)` — registers a socket for push-based reads.
/// Called from JS after the 'connect' event (parser is ready to receive data).
pub fn tcp_enable_polling_callback(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    _ret: v8::ReturnValue,
) {
    if args.length() < 1 {
        return;
    }
    let conn_id = args.get(0).uint32_value(scope).unwrap_or(0);
    TCP_POLL_SET.with(|s| {
        s.borrow_mut().insert(conn_id);
    });
    debug!(conn_id, "TCP polling enabled");
}

/// `__rex_tcp_disable_polling(connId)` — removes a socket from the poll set.
pub fn tcp_disable_polling_callback(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    _ret: v8::ReturnValue,
) {
    if args.length() < 1 {
        return;
    }
    let conn_id = args.get(0).uint32_value(scope).unwrap_or(0);
    TCP_POLL_SET.with(|s| {
        s.borrow_mut().remove(&conn_id);
    });
}

/// `__rex_tcp_close(connId)` — closes a TCP connection.
pub fn tcp_close_callback(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    _ret: v8::ReturnValue,
) {
    if args.length() < 1 {
        return;
    }

    let conn_id = args.get(0).uint32_value(scope).unwrap_or(0);

    TCP_POLL_SET.with(|s| {
        s.borrow_mut().remove(&conn_id);
    });
    TCP_CONNECTIONS.with(|conns| {
        if conns.borrow_mut().remove(&conn_id).is_some() {
            debug!(conn_id, "TCP connection closed");
        }
    });
}

/// `__rex_tcp_debug(message)` — logs a JS debug message via tracing.
pub fn tcp_debug_callback(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    _ret: v8::ReturnValue,
) {
    if args.length() < 1 {
        return;
    }
    let msg = args.get(0).to_rust_string_lossy(scope);
    eprintln!("[JS] {}", msg);
}

// ── Push-based polling ────────────────────────────────────────────────────

/// Read result from a non-blocking TCP poll.
struct PollResult {
    conn_id: u32,
    kind: PollResultKind,
}

enum PollResultKind {
    Data(Vec<u8>),
    Eof,
    Error(String),
}

/// Poll all registered TCP sockets with non-blocking reads and push data to JS.
///
/// For each socket in [`TCP_POLL_SET`]:
/// - Tries a non-blocking read
/// - If data: calls `globalThis.__rex_tcp_push_data(connId, Uint8Array)`
/// - If EOF: calls `globalThis.__rex_tcp_push_eof(connId)`
/// - If error: calls `globalThis.__rex_tcp_push_error(connId, message)`
/// - If WouldBlock: skip (no data available yet)
///
/// Returns `true` if any data was pushed (progress was made).
pub fn poll_tcp_sockets(isolate: &mut v8::OwnedIsolate, context: &v8::Global<v8::Context>) -> bool {
    let poll_ids: Vec<u32> = TCP_POLL_SET.with(|s| s.borrow().iter().copied().collect());
    if poll_ids.is_empty() {
        return false;
    }

    // Phase 1: Non-blocking reads (no V8 scope needed)
    let mut results: Vec<PollResult> = Vec::new();

    TCP_CONNECTIONS.with(|conns| {
        let mut conns = conns.borrow_mut();
        for &conn_id in &poll_ids {
            let stream = match conns.get_mut(&conn_id) {
                Some(s) => s,
                None => {
                    // Connection was closed but still in poll set
                    results.push(PollResult {
                        conn_id,
                        kind: PollResultKind::Eof,
                    });
                    continue;
                }
            };

            let _ = stream.set_nonblocking(true);
            let mut buf = vec![0u8; 16384];
            let read_result = stream.read(&mut buf);
            let _ = stream.set_nonblocking(false);

            match read_result {
                Ok(0) => {
                    results.push(PollResult {
                        conn_id,
                        kind: PollResultKind::Eof,
                    });
                }
                Ok(n) => {
                    buf.truncate(n);
                    results.push(PollResult {
                        conn_id,
                        kind: PollResultKind::Data(buf),
                    });
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    // No data available — skip
                }
                Err(e) => {
                    results.push(PollResult {
                        conn_id,
                        kind: PollResultKind::Error(format!("{e}")),
                    });
                }
            }
        }
    });

    if results.is_empty() {
        return false;
    }

    // Remove EOF/error connections from poll set
    for result in &results {
        if matches!(result.kind, PollResultKind::Eof | PollResultKind::Error(_)) {
            TCP_POLL_SET.with(|s| {
                s.borrow_mut().remove(&result.conn_id);
            });
        }
    }

    // Phase 2: Push results to JS via global callbacks (with exception handling)
    v8::scope_with_context!(scope, isolate, context);
    v8::tc_scope!(tc, scope);
    let global = context.open(tc).global(tc);

    for result in results {
        let conn_id = result.conn_id;
        match result.kind {
            PollResultKind::Data(data) => {
                let fn_key = v8::String::new(tc, "__rex_tcp_push_data").expect("v8 string");
                if let Some(func_val) = global.get(tc, fn_key.into()) {
                    if let Ok(func) = v8::Local::<v8::Function>::try_from(func_val) {
                        let conn_id_val = v8::Integer::new(tc, conn_id as i32);
                        let store = v8::ArrayBuffer::new_backing_store_from_vec(data).make_shared();
                        let ab = v8::ArrayBuffer::with_backing_store(tc, &store);
                        let uint8 = v8::Uint8Array::new(tc, ab, 0, ab.byte_length())
                            .expect("v8 Uint8Array");
                        let recv = v8::undefined(tc);
                        let r = func.call(tc, recv.into(), &[conn_id_val.into(), uint8.into()]);
                        if r.is_none() {
                            if let Some(exc) = tc.exception() {
                                let msg = exc.to_rust_string_lossy(tc);
                                debug!(conn_id, "TCP push_data exception: {}", msg);
                            }
                            tc.reset();
                        }
                    }
                } else {
                    debug!(conn_id, "__rex_tcp_push_data not found on globalThis");
                }
            }
            PollResultKind::Eof => {
                let fn_key = v8::String::new(tc, "__rex_tcp_push_eof").expect("v8 string");
                if let Some(func_val) = global.get(tc, fn_key.into()) {
                    if let Ok(func) = v8::Local::<v8::Function>::try_from(func_val) {
                        let conn_id_val = v8::Integer::new(tc, conn_id as i32);
                        let recv = v8::undefined(tc);
                        let r = func.call(tc, recv.into(), &[conn_id_val.into()]);
                        if r.is_none() {
                            if let Some(exc) = tc.exception() {
                                let msg = exc.to_rust_string_lossy(tc);
                                debug!(conn_id, "TCP push_eof exception: {}", msg);
                            }
                            tc.reset();
                        }
                    }
                }
            }
            PollResultKind::Error(msg) => {
                let fn_key = v8::String::new(tc, "__rex_tcp_push_error").expect("v8 string");
                if let Some(func_val) = global.get(tc, fn_key.into()) {
                    if let Ok(func) = v8::Local::<v8::Function>::try_from(func_val) {
                        let conn_id_val = v8::Integer::new(tc, conn_id as i32);
                        let msg_val = v8::String::new(tc, &msg).expect("v8 string");
                        let recv = v8::undefined(tc);
                        let r = func.call(tc, recv.into(), &[conn_id_val.into(), msg_val.into()]);
                        if r.is_none() {
                            if let Some(exc) = tc.exception() {
                                let emsg = exc.to_rust_string_lossy(tc);
                                debug!(conn_id, "TCP push_error exception: {}", emsg);
                            }
                            tc.reset();
                        }
                    }
                }
            }
        }
    }

    true
}

/// Register all `__rex_tcp_*` callbacks on the V8 global object.
pub fn register_tcp_callbacks(
    scope: &mut v8::ContextScope<v8::HandleScope>,
    global: v8::Local<v8::Object>,
) -> Result<()> {
    macro_rules! register_fn {
        ($name:expr, $callback:expr) => {{
            let t = v8::FunctionTemplate::new(scope, $callback);
            let f = t
                .get_function(scope)
                .ok_or_else(|| anyhow::anyhow!("Failed to create {}", $name))?;
            let k = v8::String::new(scope, $name)
                .ok_or_else(|| anyhow::anyhow!("V8 string alloc failed for '{}'", $name))?;
            global.set(scope, k.into(), f.into());
        }};
    }

    register_fn!("__rex_tcp_connect", tcp_connect_callback);
    register_fn!("__rex_tcp_write", tcp_write_callback);
    register_fn!("__rex_tcp_start_tls", tcp_start_tls_callback);
    register_fn!("__rex_tcp_close", tcp_close_callback);
    register_fn!("__rex_tcp_enable_polling", tcp_enable_polling_callback);
    register_fn!("__rex_tcp_disable_polling", tcp_disable_polling_callback);
    register_fn!("__rex_tcp_debug", tcp_debug_callback);

    debug!("Registered TCP callbacks on globalThis");
    Ok(())
}
