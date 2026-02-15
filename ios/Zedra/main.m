// Zedra iOS — Obj-C App Delegate for GPUI Metal Rendering
//
// Lifecycle:
//   1. gpui_ios_initialize()         — set up GPUI FFI state
//   2. zedra_launch_gpui()           — create GPUI Application + register callback
//   3. gpui_ios_did_finish_launching — invoke callback → opens Metal window
//   4. CADisplayLink                 — drives gpui_ios_request_frame() at 60 FPS

#import <UIKit/UIKit.h>

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
    NSLog(@"Zedra: Launching...");

    // 1. Initialize GPUI FFI state (sets up IOS_APP_STATE)
    self.gpuiApp = gpui_ios_initialize();
    NSLog(@"Zedra: GPUI initialized: %p", self.gpuiApp);

    // 2. Create the GPUI Application and register the window-open callback
    zedra_launch_gpui();
    NSLog(@"Zedra: GPUI app created");

    // 3. Invoke the finish-launching callback (opens the Metal window)
    gpui_ios_did_finish_launching(self.gpuiApp);
    NSLog(@"Zedra: Finish launching callback invoked");

    // 4. Get the GPUI window pointer
    self.gpuiWindow = gpui_ios_get_window();
    if (self.gpuiWindow) {
        NSLog(@"Zedra: Got GPUI window: %p", self.gpuiWindow);

        // 5. Start CADisplayLink to drive rendering at 60 FPS
        self.displayLink = [CADisplayLink displayLinkWithTarget:self
                                                       selector:@selector(renderFrame)];
        [self.displayLink addToRunLoop:[NSRunLoop mainRunLoop]
                               forMode:NSRunLoopCommonModes];
        NSLog(@"Zedra: CADisplayLink started");
    } else {
        NSLog(@"Zedra: WARNING — no GPUI window created");
    }

    NSLog(@"Zedra: Launch complete");
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
    @autoreleasepool {
        return UIApplicationMain(argc, argv, nil,
                                 NSStringFromClass([ZedraAppDelegate class]));
    }
}
