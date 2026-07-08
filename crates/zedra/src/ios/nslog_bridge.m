#import <Foundation/Foundation.h>

// Routes Rust log output through NSLog (unified logging) for Console.app.
// This is one of two intentionally duplicate channels — see the doc comment
// at the top of logger.rs for why (short version: idevicesyslog can't decode
// our messages here regardless of this fix, so automation reads stderr instead).
// %{public}s: without it, iOS redacts the message to `<private>` in Console.app/
// idevicesyslog/any syslog client — the redaction happens in the OS logging
// daemon itself, so no client-side tool (pymobiledevice3 included) recovers it.
void zedra_nslog(const char *msg) {
    if (msg == NULL) {
        return;
    }

    NSLog(@"%{public}s", msg);
}
