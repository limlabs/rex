//! TCP socket support for V8 isolates.
//!
//! Provides `globalThis.__rex_tcp_*` callbacks for the `cloudflare:sockets` polyfill.
//! Uses a batching model similar to [`crate::fetch`]: synchronous connect/write/close
//! and async reads that return promises resolved in the IO loop.

use anyhow::Result;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
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

/// A queued TCP read request waiting to be resolved.
pub struct TcpReadRequest {
    pub resolver: v8::Global<v8::PromiseResolver>,
    pub conn_id: u32,
}

/// Result type for resolved TCP reads.
type TcpResolveEntry = (v8::Global<v8::PromiseResolver>, Result<Vec<u8>, String>);

thread_local! {
    /// Active TCP connections on this isolate thread.
    static TCP_CONNECTIONS: RefCell<HashMap<u32, TcpStream>> = RefCell::new(HashMap::new());

    /// Queue of pending TCP read requests.
    pub static TCP_READ_QUEUE: RefCell<Vec<TcpReadRequest>> = const { RefCell::new(Vec::new()) };

    /// Next connection ID.
    static NEXT_TCP_ID: Cell<u32> = const { Cell::new(1) };

    /// Shared TLS client config (lazily initialized).
    static TLS_CONFIG: RefCell<Option<std::sync::Arc<rustls::ClientConfig>>> = const { RefCell::new(None) };
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

/// `__rex_tcp_connect(host, port)` → conn_id (u32)
///
/// Opens a TCP connection synchronously (blocks on std::net).
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
            // Set TCP_NODELAY for database connections (reduces latency)
            let _ = stream.set_nodelay(true);
            // Set a read timeout to prevent blocking forever
            let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(30)));

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
///
/// Writes data to a TCP connection synchronously. `data` can be a string or Uint8Array.
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
            let msg = v8::String::new(scope, &e).expect("v8 string");
            let err = v8::Exception::error(scope, msg);
            scope.throw_exception(err);
        }
    }
}

/// `__rex_tcp_read(connId)` → Promise<{done: boolean, value?: Uint8Array}>
///
/// Queues a read request. The promise is resolved in [`drain_tcp_reads`].
pub fn tcp_read_callback(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut ret: v8::ReturnValue,
) {
    if args.length() < 1 {
        ret.set(v8::undefined(scope).into());
        return;
    }

    let conn_id = args.get(0).uint32_value(scope).unwrap_or(0);

    let Some(resolver) = v8::PromiseResolver::new(scope) else {
        ret.set(v8::undefined(scope).into());
        return;
    };
    let promise = resolver.get_promise(scope);

    let global_resolver = v8::Global::new(scope, resolver);
    TCP_READ_QUEUE.with(|q| {
        q.borrow_mut().push(TcpReadRequest {
            resolver: global_resolver,
            conn_id,
        });
    });

    ret.set(promise.into());
}

/// `__rex_tcp_start_tls(connId, hostname)` → new conn_id (u32)
///
/// Upgrades a plain TCP connection to TLS. Returns a new connection ID.
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

/// `__rex_tcp_close(connId)` → void
///
/// Closes a TCP connection and removes it from the connection map.
pub fn tcp_close_callback(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    _ret: v8::ReturnValue,
) {
    if args.length() < 1 {
        return;
    }

    let conn_id = args.get(0).uint32_value(scope).unwrap_or(0);

    TCP_CONNECTIONS.with(|conns| {
        if conns.borrow_mut().remove(&conn_id).is_some() {
            debug!(conn_id, "TCP connection closed");
        }
    });
}

/// Drain the TCP read queue and resolve promises.
///
/// For each pending read, attempts a non-blocking read from the socket.
/// Returns the number of reads that are still pending (connection exists but no data yet).
pub fn drain_tcp_reads(isolate: &mut v8::OwnedIsolate, context: &v8::Global<v8::Context>) -> usize {
    let pending: Vec<TcpReadRequest> = TCP_READ_QUEUE.with(|q| q.borrow_mut().drain(..).collect());

    if pending.is_empty() {
        return 0;
    }

    let mut still_pending = Vec::new();
    let mut to_resolve: Vec<TcpResolveEntry> = Vec::new();

    for req in pending {
        let result = TCP_CONNECTIONS.with(|conns| {
            let mut conns = conns.borrow_mut();
            let stream = match conns.get_mut(&req.conn_id) {
                Some(s) => s,
                None => return Err("closed".to_string()),
            };

            // Set non-blocking for the read attempt
            let _ = stream.set_nonblocking(true);
            let mut buf = vec![0u8; 8192];
            let result = stream.read(&mut buf);
            let _ = stream.set_nonblocking(false);

            match result {
                Ok(0) => Err("eof".to_string()),
                Ok(n) => {
                    buf.truncate(n);
                    Ok(buf)
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    // No data available yet — try blocking read
                    let _ = stream.set_nonblocking(false);
                    match stream.read(&mut buf) {
                        Ok(0) => Err("eof".to_string()),
                        Ok(n) => {
                            buf.truncate(n);
                            Ok(buf)
                        }
                        Err(e) => Err(format!("TCP read error: {e}")),
                    }
                }
                Err(e) => Err(format!("TCP read error: {e}")),
            }
        });

        match &result {
            Err(e) if e == "retry" => {
                still_pending.push(req);
            }
            _ => {
                to_resolve.push((req.resolver, result));
            }
        }
    }

    // Re-queue still-pending reads
    if !still_pending.is_empty() {
        TCP_READ_QUEUE.with(|q| {
            q.borrow_mut().extend(still_pending);
        });
    }

    let remaining = TCP_READ_QUEUE.with(|q| q.borrow().len());

    // Resolve/reject promises
    if !to_resolve.is_empty() {
        v8::scope_with_context!(scope, isolate, context);

        for (resolver_global, result) in to_resolve {
            let resolver = v8::Local::new(scope, &resolver_global);
            match result {
                Ok(data) => {
                    // Create { done: false, value: Uint8Array }
                    let obj = v8::Object::new(scope);
                    let done_key = v8::String::new(scope, "done").expect("v8 string");
                    let value_key = v8::String::new(scope, "value").expect("v8 string");
                    let done_val = v8::Boolean::new(scope, false);
                    obj.set(scope, done_key.into(), done_val.into());

                    let store = v8::ArrayBuffer::new_backing_store_from_vec(data).make_shared();
                    let ab = v8::ArrayBuffer::with_backing_store(scope, &store);
                    let uint8 =
                        v8::Uint8Array::new(scope, ab, 0, ab.byte_length()).expect("v8 Uint8Array");
                    obj.set(scope, value_key.into(), uint8.into());

                    resolver.resolve(scope, obj.into());
                }
                Err(ref e) if e == "eof" || e == "closed" => {
                    // Create { done: true }
                    let obj = v8::Object::new(scope);
                    let done_key = v8::String::new(scope, "done").expect("v8 string");
                    let done_val = v8::Boolean::new(scope, true);
                    obj.set(scope, done_key.into(), done_val.into());
                    resolver.resolve(scope, obj.into());
                }
                Err(ref e) => {
                    let msg = v8::String::new(scope, e).expect("v8 string");
                    let err = v8::Exception::error(scope, msg);
                    resolver.reject(scope, err);
                }
            }
        }
    }

    remaining
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
    register_fn!("__rex_tcp_read", tcp_read_callback);
    register_fn!("__rex_tcp_start_tls", tcp_start_tls_callback);
    register_fn!("__rex_tcp_close", tcp_close_callback);

    debug!("Registered TCP callbacks on globalThis");
    Ok(())
}
