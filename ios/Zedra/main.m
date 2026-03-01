// Zedra iOS — Obj-C App Delegate for GPUI Metal Rendering
//
// Lifecycle:
//   1. gpui_ios_initialize()         — set up GPUI FFI state
//   2. zedra_launch_gpui()           — create GPUI Application + register callback
//   3. gpui_ios_did_finish_launching — invoke callback → opens Metal window
//   4. CADisplayLink                 — drives gpui_ios_request_frame() at 60 FPS

#import <UIKit/UIKit.h>
#import <unistd.h>

// Synchronous write to stderr — visible in `devicectl device process launch --console`.
// Use raw write() so it works even if higher-level I/O is not yet set up.
#define DIAG(msg) do { \
    const char *_s = "ZEDRA_DIAG: " msg "\n"; \
    write(STDERR_FILENO, _s, strlen(_s)); \
} while(0)

// GPUI FFI (from gpui crate)
extern void* gpui_ios_initialize(void);
extern void gpui_ios_did_finish_launching(void* app_ptr);
extern void* gpui_ios_get_window(void);
extern void gpui_ios_request_frame(void* window_ptr);
extern void gpui_ios_will_enter_foreground(void* app_ptr);
extern void gpui_ios_did_become_active(void* app_ptr);
extern void gpui_ios_will_resign_active(void* app_ptr);
extern void gpui_ios_did_enter_background(void* app_ptr);
extern void gpui_ios_will_terminate(void* app_ptr);

// Zedra FFI (from zedra-ios crate)
extern void zedra_launch_gpui(void);

@interface ZedraAppDelegate : UIResponder <UIApplicationDelegate>
@property (nonatomic, assign) void *gpuiApp;
@property (nonatomic, assign) void *gpuiWindow;
@property (nonatomic, strong) CADisplayLink *displayLink;
@end

@implementation ZedraAppDelegate

- (BOOL)application:(UIApplication *)application
    didFinishLaunchingWithOptions:(NSDictionary *)launchOptions {
    DIAG("didFinishLaunching start");

    // 1. Initialize GPUI FFI state (sets up IOS_APP_STATE)
    self.gpuiApp = gpui_ios_initialize();
    DIAG("gpui_ios_initialize done");

    // 2. Create the GPUI Application and register the window-open callback
    DIAG("calling zedra_launch_gpui");
    zedra_launch_gpui();
    DIAG("zedra_launch_gpui done");

    // 3. Invoke the finish-launching callback (opens the Metal window)
    DIAG("calling gpui_ios_did_finish_launching");
    gpui_ios_did_finish_launching(self.gpuiApp);
    DIAG("gpui_ios_did_finish_launching done");

    // 4. Get the GPUI window pointer
    self.gpuiWindow = gpui_ios_get_window();
    if (self.gpuiWindow) {
        DIAG("got GPUI window, starting CADisplayLink");

        // 5. Start CADisplayLink to drive rendering at 60 FPS
        self.displayLink = [CADisplayLink displayLinkWithTarget:self
                                                       selector:@selector(renderFrame)];
        [self.displayLink addToRunLoop:[NSRunLoop mainRunLoop]
                               forMode:NSRunLoopCommonModes];
        DIAG("CADisplayLink started");
    } else {
        DIAG("WARNING: no GPUI window created");
    }

    DIAG("launch complete");
    return YES;
}

- (void)renderFrame {
    if (self.gpuiWindow) {
        gpui_ios_request_frame(self.gpuiWindow);
    }
}

- (void)applicationWillEnterForeground:(UIApplication *)application {
    gpui_ios_will_enter_foreground(self.gpuiApp);

    // Resume display link
    if (!self.displayLink && self.gpuiWindow) {
        self.displayLink = [CADisplayLink displayLinkWithTarget:self
                                                       selector:@selector(renderFrame)];
        [self.displayLink addToRunLoop:[NSRunLoop mainRunLoop]
                               forMode:NSRunLoopCommonModes];
    }
}

- (void)applicationDidBecomeActive:(UIApplication *)application {
    gpui_ios_did_become_active(self.gpuiApp);
}

- (void)applicationWillResignActive:(UIApplication *)application {
    gpui_ios_will_resign_active(self.gpuiApp);
}

- (void)applicationDidEnterBackground:(UIApplication *)application {
    gpui_ios_did_enter_background(self.gpuiApp);

    // Pause display link to save power
    if (self.displayLink) {
        [self.displayLink invalidate];
        self.displayLink = nil;
    }
}

- (void)applicationWillTerminate:(UIApplication *)application {
    if (self.displayLink) {
        [self.displayLink invalidate];
        self.displayLink = nil;
    }
    gpui_ios_will_terminate(self.gpuiApp);
}

@end

int main(int argc, char * argv[]) {
    DIAG("main() entered");
    @autoreleasepool {
        DIAG("calling UIApplicationMain");
        return UIApplicationMain(argc, argv, nil,
                                 NSStringFromClass([ZedraAppDelegate class]));
    }
}
