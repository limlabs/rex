#![allow(clippy::unwrap_used)]

mod common;

use common::{make_isolate, make_isolate_ext, make_server_bundle, TestPage, MOCK_REACT_RUNTIME};
use rex_v8::SsrIsolate;

#[test]
fn test_render_simple_page() {
    let mut iso = make_isolate(&[(
        "index",
        "function Index() { return React.createElement('h1', null, 'Hello'); }",
        None,
    )]);
    let result = iso.render_page("index", "{}").unwrap();
    assert_eq!(result.body, "<h1>Hello</h1>");
    assert_eq!(result.head, "");
}

#[test]
fn test_render_with_props() {
    let mut iso = make_isolate(&[(
        "greet",
        "function Greet(props) { return React.createElement('p', null, 'Hi ' + props.name); }",
        None,
    )]);
    let result = iso.render_page("greet", r#"{"name":"Rex"}"#).unwrap();
    assert_eq!(result.body, "<p>Hi Rex</p>");
}

#[test]
fn test_render_nested_elements() {
    let mut iso = make_isolate(&[(
        "nested",
        r#"function Page() {
            return React.createElement('div', {class: 'wrapper'},
                React.createElement('h1', null, 'Title'),
                React.createElement('p', null, 'Body')
            );
        }"#,
        None,
    )]);
    let result = iso.render_page("nested", "{}").unwrap();
    assert_eq!(
        result.body,
        r#"<div class="wrapper"><h1>Title</h1><p>Body</p></div>"#
    );
}

#[test]
fn test_render_missing_page() {
    let mut iso = make_isolate(&[]);
    let err = iso.render_page("nonexistent", "{}").unwrap_err();
    assert!(
        err.to_string().contains("Page not found"),
        "expected 'Page not found', got: {err}"
    );
}

#[test]
fn test_render_component_throws() {
    let mut iso = make_isolate(&[(
        "bad",
        "function Bad() { throw new Error('component broke'); }",
        None,
    )]);
    let err = iso.render_page("bad", "{}").unwrap_err();
    assert!(
        err.to_string().contains("component broke"),
        "expected 'component broke', got: {err}"
    );
}

#[test]
fn test_gssp_sync() {
    let mut iso = make_isolate(&[(
        "page",
        "function Page(props) { return React.createElement('span', null, props.title); }",
        Some("function(ctx) { return { props: { title: 'from gssp' } }; }"),
    )]);
    let json = iso
        .get_server_side_props("page", r#"{"params":{},"query":{}}"#)
        .unwrap();
    let val: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(val["props"]["title"], "from gssp");
}

#[test]
fn test_gssp_no_gssp_returns_empty_props() {
    let mut iso = make_isolate(&[(
        "page",
        "function Page() { return React.createElement('div', null, 'hi'); }",
        None,
    )]);
    let json = iso
        .get_server_side_props("page", r#"{"params":{},"query":{}}"#)
        .unwrap();
    let val: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(val["props"], serde_json::json!({}));
}

#[test]
fn test_gssp_receives_context() {
    let mut iso = make_isolate(&[(
        "page",
        "function Page() { return React.createElement('div'); }",
        Some(
            "function(ctx) { return { props: { slug: ctx.params.slug, url: ctx.resolved_url } }; }",
        ),
    )]);
    let context = r#"{"params":{"slug":"hello"},"query":{},"resolved_url":"/blog/hello","headers":{},"cookies":{}}"#;
    let json = iso.get_server_side_props("page", context).unwrap();
    let val: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(val["props"]["slug"], "hello");
    assert_eq!(val["props"]["url"], "/blog/hello");
}

#[test]
fn test_gssp_async() {
    let mut iso = make_isolate(&[(
        "page",
        "function Page() { return React.createElement('div'); }",
        Some("function(ctx) { return Promise.resolve({ props: { async: true } }); }"),
    )]);
    let json = iso
        .get_server_side_props("page", r#"{"params":{},"query":{}}"#)
        .unwrap();
    let val: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(val["props"]["async"], true);
}

#[test]
fn test_gssp_throws() {
    let mut iso = make_isolate(&[(
        "page",
        "function Page() { return React.createElement('div'); }",
        Some("function(ctx) { throw new Error('gssp failed'); }"),
    )]);
    let err = iso
        .get_server_side_props("page", r#"{"params":{},"query":{}}"#)
        .unwrap_err();
    assert!(
        err.to_string().contains("gssp failed"),
        "expected 'gssp failed', got: {err}"
    );
}

#[test]
fn test_gssp_missing_page() {
    let mut iso = make_isolate(&[]);
    let json = iso
        .get_server_side_props("nonexistent", r#"{"params":{},"query":{}}"#)
        .unwrap();
    let val: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(val["props"], serde_json::json!({}));
}

#[test]
fn test_reload_replaces_pages() {
    let mut iso = make_isolate(&[(
        "page",
        "function Page() { return React.createElement('p', null, 'v1'); }",
        None,
    )]);
    assert_eq!(iso.render_page("page", "{}").unwrap().body, "<p>v1</p>");

    let new_bundle = make_server_bundle(&[(
        "page",
        "function Page() { return React.createElement('p', null, 'v2'); }",
        None,
    )]);
    iso.reload(&new_bundle).unwrap();
    assert_eq!(iso.render_page("page", "{}").unwrap().body, "<p>v2</p>");
}

#[test]
fn test_reload_adds_new_pages() {
    let mut iso = make_isolate(&[(
        "page",
        "function Page() { return React.createElement('p', null, 'original'); }",
        None,
    )]);

    let new_bundle = make_server_bundle(&[
        (
            "page",
            "function Page() { return React.createElement('p', null, 'original'); }",
            None,
        ),
        (
            "about",
            "function About() { return React.createElement('h1', null, 'About'); }",
            None,
        ),
    ]);
    iso.reload(&new_bundle).unwrap();
    assert_eq!(
        iso.render_page("about", "{}").unwrap().body,
        "<h1>About</h1>"
    );
}

#[test]
fn test_invalid_server_bundle() {
    rex_v8::init_v8();
    let result = SsrIsolate::new("this is not valid javascript {{{{", None);
    assert!(result.is_err());
}

#[test]
fn test_multiple_renders_same_isolate() {
    let mut iso = make_isolate(&[(
        "page",
        "function Page(props) { return React.createElement('b', null, props.n); }",
        None,
    )]);
    for i in 0..5 {
        let result = iso.render_page("page", &format!(r#"{{"n":{i}}}"#)).unwrap();
        assert_eq!(result.body, format!("<b>{i}</b>"));
    }
}

#[test]
fn test_render_with_head_elements() {
    let mut iso = make_isolate(&[(
        "seo",
        r#"function SeoPage(props) {
            var Head = globalThis.__rex_head_component;
            return React.createElement('div', null,
                React.createElement(Head, null,
                    React.createElement('title', null, props.title),
                    React.createElement('meta', { name: 'description', content: 'A test page' })
                ),
                React.createElement('h1', null, props.title)
            );
        }"#,
        None,
    )]);
    let result = iso.render_page("seo", r#"{"title":"My Page"}"#).unwrap();
    assert!(
        result.body.contains("<h1>My Page</h1>"),
        "body should have h1: {}",
        result.body
    );
    assert!(
        !result.body.contains("<title>"),
        "body should NOT contain title: {}",
        result.body
    );
    assert!(
        result.head.contains("<title>My Page</title>"),
        "head should contain title: {}",
        result.head
    );
    assert!(
        result.head.contains("description"),
        "head should contain meta description: {}",
        result.head
    );
}

#[test]
fn test_gsp_sync() {
    let mut iso = make_isolate_ext(&[TestPage {
        key: "page",
        component:
            "function Page(props) { return React.createElement('span', null, props.title); }",
        gssp: None,
        gsp: Some("function(ctx) { return { props: { title: 'from gsp' } }; }"),
    }]);
    let json = iso.get_static_props("page", r#"{"params":{}}"#).unwrap();
    let val: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(val["props"]["title"], "from gsp");
}

#[test]
fn test_gsp_async() {
    let mut iso = make_isolate_ext(&[TestPage {
        key: "page",
        component: "function Page() { return React.createElement('div'); }",
        gssp: None,
        gsp: Some("function(ctx) { return Promise.resolve({ props: { async: true } }); }"),
    }]);
    let json = iso.get_static_props("page", r#"{"params":{}}"#).unwrap();
    let val: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(val["props"]["async"], true);
}

#[test]
fn test_render_suspense_renders_children() {
    let mut iso = make_isolate(&[(
        "page",
        r#"function Page() {
            return React.createElement(React.Suspense, { fallback: 'Loading' },
                React.createElement('div', null, 'Suspense child content')
            );
        }"#,
        None,
    )]);
    let result = iso.render_page("page", "{}").unwrap();
    assert!(
        result.body.contains("Suspense child content"),
        "should render children, not fallback: {}",
        result.body
    );
    assert!(
        !result.body.contains("Loading"),
        "should NOT render fallback when children render normally: {}",
        result.body
    );
}

#[test]
fn test_render_suspense_fallback_on_throw() {
    let mut iso = make_isolate(&[(
        "page",
        r#"function Page() {
            function Thrower() { throw Promise.resolve(); }
            return React.createElement(React.Suspense, { fallback: 'Loading...' },
                React.createElement(Thrower)
            );
        }"#,
        None,
    )]);
    let result = iso.render_page("page", "{}").unwrap();
    assert!(
        result.body.contains("Loading..."),
        "should render fallback when child throws a promise: {}",
        result.body
    );
}

#[test]
fn test_head_reset_between_renders() {
    let mut iso = make_isolate(&[
        (
            "page1",
            r#"function Page1() {
                var Head = globalThis.__rex_head_component;
                return React.createElement('div', null,
                    React.createElement(Head, null, React.createElement('title', null, 'Page 1'))
                );
            }"#,
            None,
        ),
        (
            "page2",
            r#"function Page2() {
                return React.createElement('div', null, 'No head');
            }"#,
            None,
        ),
    ]);
    let r1 = iso.render_page("page1", "{}").unwrap();
    assert!(
        r1.head.contains("<title>Page 1</title>"),
        "page1 should have title"
    );

    let r2 = iso.render_page("page2", "{}").unwrap();
    assert_eq!(
        r2.head, "",
        "page2 should have empty head (no leak from page1)"
    );
}

#[test]
fn test_reload_updates_pages() {
    let mut iso = make_isolate(&[(
        "index",
        "function Index() { return React.createElement('p', null, 'v1'); }",
        None,
    )]);
    let r1 = iso.render_page("index", "{}").unwrap();
    assert_eq!(r1.body, "<p>v1</p>");

    let new_bundle = format!(
        "{}\n{}",
        MOCK_REACT_RUNTIME,
        make_server_bundle(&[(
            "index",
            "function Index() { return React.createElement('p', null, 'v2'); }",
            None,
        )])
    );
    iso.reload(&new_bundle).unwrap();
    let r2 = iso.render_page("index", "{}").unwrap();
    assert_eq!(r2.body, "<p>v2</p>");
}

#[test]
fn test_reload_bad_bundle_restores_previous() {
    let mut iso = make_isolate(&[(
        "index",
        "function Index() { return React.createElement('p', null, 'ok'); }",
        None,
    )]);
    let r1 = iso.render_page("index", "{}").unwrap();
    assert_eq!(r1.body, "<p>ok</p>");

    let result = iso.reload("this is not valid javascript {{{{");
    assert!(result.is_err(), "reload should fail for bad JS");

    let r2 = iso.render_page("index", "{}").unwrap();
    assert_eq!(r2.body, "<p>ok</p>");
}

#[test]
fn test_process_env_from_rust() {
    std::env::set_var("REX_TEST_POLYFILL", "hello_from_rust");

    let mut iso = make_isolate(&[(
        "envtest",
        "function EnvTest() { return React.createElement('p', null, process.env.REX_TEST_POLYFILL || 'MISSING'); }",
        Some("function(ctx) { return { props: { val: process.env.REX_TEST_POLYFILL } }; }"),
    )]);

    let gssp_result = iso.get_server_side_props("envtest", "{}").unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&gssp_result).unwrap();
    assert_eq!(parsed["props"]["val"], "hello_from_rust");

    let render = iso.render_page("envtest", "{}").unwrap();
    assert!(
        render.body.contains("hello_from_rust"),
        "SSR body should contain env var value, got: {}",
        render.body
    );

    std::env::remove_var("REX_TEST_POLYFILL");
}

#[test]
fn test_process_env_is_writable() {
    let mut iso = make_isolate(&[(
        "writetest",
        "function WriteTest() { process.env.DYNAMIC = 'set_at_runtime'; return React.createElement('p', null, process.env.DYNAMIC); }",
        None,
    )]);
    let render = iso.render_page("writetest", "{}").unwrap();
    assert!(
        render.body.contains("set_at_runtime"),
        "process.env should be writable, got: {}",
        render.body
    );
}

#[test]
fn test_console_log_emits_tracing_event() {
    use std::sync::{Arc, Mutex};
    use tracing_subscriber::layer::SubscriberExt;

    struct CaptureLayer {
        messages: Arc<Mutex<Vec<(tracing::Level, String)>>>,
    }

    impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for CaptureLayer {
        fn on_event(
            &self,
            event: &tracing::Event<'_>,
            _ctx: tracing_subscriber::layer::Context<'_, S>,
        ) {
            struct Visitor(String);
            impl tracing::field::Visit for Visitor {
                fn record_debug(
                    &mut self,
                    field: &tracing::field::Field,
                    value: &dyn std::fmt::Debug,
                ) {
                    if field.name() == "message" {
                        self.0 = format!("{value:?}");
                    }
                }
            }
            let mut visitor = Visitor(String::new());
            event.record(&mut visitor);
            self.messages
                .lock()
                .unwrap()
                .push((*event.metadata().level(), visitor.0));
        }
    }

    let messages = Arc::new(Mutex::new(Vec::new()));
    let subscriber = tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::new("v8::console=info"))
        .with(CaptureLayer {
            messages: messages.clone(),
        });

    let _guard = tracing::subscriber::set_default(subscriber);

    let mut iso = make_isolate(&[(
        "logpage",
        r#"function LogPage() {
            console.log("hello from ssr");
            console.warn("warning from ssr");
            console.error("error from ssr");
            return React.createElement('p', null, 'logged');
        }"#,
        None,
    )]);
    let render = iso.render_page("logpage", "{}").unwrap();
    assert!(render.body.contains("logged"), "page should render");

    let captured = messages.lock().unwrap();
    assert!(
        captured
            .iter()
            .any(|(_, msg)| msg.contains("hello from ssr")),
        "console.log should emit tracing event, captured: {captured:?}"
    );
    assert!(
        captured
            .iter()
            .any(|(level, msg)| *level == tracing::Level::WARN && msg.contains("warning from ssr")),
        "console.warn should emit WARN-level event, captured: {captured:?}"
    );
    assert!(
        captured
            .iter()
            .any(|(level, msg)| *level == tracing::Level::ERROR && msg.contains("error from ssr")),
        "console.error should emit ERROR-level event, captured: {captured:?}"
    );
}
