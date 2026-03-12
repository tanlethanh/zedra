// ZedraFirebase.m — Firebase Analytics + Crashlytics bridge
//
// Provides C-linkage functions called from Rust via extern "C" declarations in
// crates/zedra/src/ios/analytics.rs.  All Firebase SDK calls are confined here
// so the Rust crate has no direct dependency on Firebase headers.
//
// Initialization:
//   zedra_firebase_initialize() must be called once before any other function.
//   main.m calls it at the top of didFinishLaunchingWithOptions:, before
//   zedra_launch_gpui().

#import <Foundation/Foundation.h>
#import <FirebaseCore/FirebaseCore.h>
#import <FirebaseAnalytics/FirebaseAnalytics.h>
#import <FirebaseCrashlytics/FirebaseCrashlytics.h>

void zedra_firebase_initialize(void) {
    [FIRApp configure];
}

void zedra_log_event(const char *name,
                     const char *const *keys,
                     const char *const *values,
                     int count)
{
    if (!name) return;
    NSString *eventName = [NSString stringWithUTF8String:name];
    NSMutableDictionary<NSString *, NSString *> *params = [NSMutableDictionary dictionary];
    for (int i = 0; i < count; i++) {
        if (!keys[i] || !values[i]) continue;
        NSString *k = [NSString stringWithUTF8String:keys[i]];
        NSString *v = [NSString stringWithUTF8String:values[i]];
        params[k] = v;
    }
    [FIRAnalytics logEventWithName:eventName parameters:params];
}

void zedra_record_error(const char *message, const char *file, int line) {
    if (!message) return;
    NSString *msg = [NSString stringWithFormat:@"[%s:%d] %s",
                     file ? file : "unknown", line, message];
    NSError *error = [NSError errorWithDomain:@"dev.zedra.rust"
                                         code:1
                                     userInfo:@{NSLocalizedDescriptionKey: msg}];
    [[FIRCrashlytics crashlytics] recordError:error];
}

void zedra_record_panic(const char *message, const char *location) {
    if (!message) return;
    NSString *loc = location ? [NSString stringWithUTF8String:location] : @"unknown";
    NSString *msg = [NSString stringWithUTF8String:message];
    NSString *full = [NSString stringWithFormat:@"Rust panic at %@: %@", loc, msg];
    [[FIRCrashlytics crashlytics] log:full];
    NSError *error = [NSError errorWithDomain:@"dev.zedra.rust.panic"
                                         code:2
                                     userInfo:@{NSLocalizedDescriptionKey: full}];
    [[FIRCrashlytics crashlytics] recordError:error];
}

void zedra_set_user_id(const char *user_id) {
    if (!user_id) return;
    NSString *uid = [NSString stringWithUTF8String:user_id];
    [FIRAnalytics setUserID:uid];
    [[FIRCrashlytics crashlytics] setUserID:uid];
}

void zedra_set_custom_key(const char *key, const char *value) {
    if (!key || !value) return;
    NSString *k = [NSString stringWithUTF8String:key];
    NSString *v = [NSString stringWithUTF8String:value];
    [[FIRCrashlytics crashlytics] setCustomValue:v forKey:k];
}
