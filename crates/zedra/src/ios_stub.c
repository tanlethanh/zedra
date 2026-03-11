// Weak stubs for Obj-C functions called from Rust on iOS.
//
// Purpose: allows the Rust cdylib to link without errors when targeting iOS.
// Real implementations live in Xcode Obj-C sources and override these at Xcode link time.
// __attribute__((weak)) ensures the linker always prefers the strong Obj-C definition.
__attribute__((weak)) void ios_present_qr_scanner(void) {}
__attribute__((weak)) const char* ios_get_documents_directory(void) { return 0; }
__attribute__((weak)) void ios_present_alert(
    unsigned int callback_id,
    const char *title,
    const char *message,
    int button_count,
    const char **labels,
    const int *styles) {}

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
