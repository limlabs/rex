use std::sync::Once;

static V8_INIT: Once = Once::new();

/// Initialize the V8 platform. Safe to call multiple times; only runs once.
pub fn init_v8() {
    V8_INIT.call_once(|| {
        let platform = v8::new_default_platform(0, false).make_shared();
        v8::V8::initialize_platform(platform);
        v8::V8::initialize();
        tracing::info!("V8 platform initialized");
    });
}
