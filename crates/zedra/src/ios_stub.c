// Weak stubs for Obj-C functions called from Rust on iOS.
//
// Purpose: allows the Rust cdylib to link without errors when targeting iOS.
// Real implementations live in Xcode Obj-C sources and override these at Xcode link time.
// __attribute__((weak)) ensures the linker always prefers the strong Obj-C definition.
__attribute__((weak)) void ios_present_qr_scanner(void) {}
__attribute__((weak)) void ios_open_url(const char *url) {}
__attribute__((weak)) const char *ios_get_documents_directory(void) { return 0; }
__attribute__((weak)) const char *ios_get_app_version(void) { return 0; }
__attribute__((weak)) const char *ios_get_app_build_number(void) { return 0; }
__attribute__((weak)) void ios_present_alert(
    unsigned int callback_id,
    const char *title,
    const char *message,
    int button_count,
    const char **labels,
    const int *styles) {}
__attribute__((weak)) void ios_present_selection(
    unsigned int callback_id,
    const char *title,
    const char *message,
    int button_count,
    const char **labels,
    const int *styles) {}
__attribute__((weak)) void ios_present_custom_sheet(
    int detent_count,
    const int *detents,
    int initial_detent,
    _Bool shows_grabber,
    _Bool expands_on_scroll_edge,
    _Bool edge_attached_in_compact_height,
    _Bool width_follows_preferred_content_size_when_edge_attached,
    _Bool has_corner_radius,
    float corner_radius,
    _Bool modal_in_presentation) {}
__attribute__((weak)) void ios_trigger_haptic(int kind) {}

// Firebase Analytics + Crashlytics stubs.
// Real implementations live in ios/Zedra/ZedraFirebase.m and override at Xcode link time.
__attribute__((weak)) void zedra_firebase_initialize(void) {}
__attribute__((weak)) void zedra_log_event(
    const char *name,
    const char *const *keys,
    const char *const *values,
    int count) {}
__attribute__((weak)) void zedra_record_error(const char *message, const char *file, int line) {}
__attribute__((weak)) void zedra_record_panic(const char *message, const char *location) {}
__attribute__((weak)) void zedra_set_user_id(const char *user_id) {}
__attribute__((weak)) void zedra_set_custom_key(const char *key, const char *value) {}
__attribute__((weak)) void zedra_set_collection_enabled(int enabled) {}
