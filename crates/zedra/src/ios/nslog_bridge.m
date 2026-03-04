#import <Foundation/Foundation.h>

// Called from Rust's IosLogger to route log output through NSLog.
// NSLog goes through ASL (Apple System Log), making it visible in
// idevicesyslog — unlike os_log which requires Console.app or log stream.
void zedra_nslog(const char *msg) {
    NSLog(@"%s", msg);
}
