#import <Foundation/Foundation.h>

// Routes Rust log output through NSLog (ASL), which idevicesyslog captures over USB.
void zedra_nslog(const char *msg) {
    if (msg == NULL) {
        return;
    }

    NSLog(@"%s", msg);
}
