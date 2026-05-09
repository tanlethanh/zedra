#import <Foundation/Foundation.h>
#import <os/log.h>

// Called from Rust's iOS logger to route log output into the device log stream.
void zedra_nslog(const char *msg) {
    if (msg == NULL) {
        return;
    }

    NSLog(@"%s", msg);
    os_log_with_type(OS_LOG_DEFAULT, OS_LOG_TYPE_INFO, "%{public}s", msg);
}
