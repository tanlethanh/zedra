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
extern void* gpui_ios_get_ui_window(void* window_ptr);
extern void gpui_ios_request_frame(void* window_ptr);
extern void gpui_ios_request_frame_forced(void* window_ptr);
extern void gpui_ios_will_enter_foreground(void* app_ptr);
extern void gpui_ios_did_become_active(void* app_ptr);
extern void gpui_ios_will_resign_active(void* app_ptr);
extern void gpui_ios_did_enter_background(void* app_ptr);
extern void gpui_ios_will_terminate(void* app_ptr);

// Zedra FFI (from zedra-ios crate)
extern void zedra_launch_gpui(void);
extern void zedra_ios_set_screen_scale(float scale);
extern void zedra_ios_set_safe_area_insets(float top, float bottom, float left, float right);
extern bool zedra_ios_check_pending_frame(void);
extern void zedra_ios_set_keyboard_height(unsigned int height_px);

@interface ZedraAppDelegate : UIResponder <UIApplicationDelegate>
@property (nonatomic, assign) void *gpuiApp;
@property (nonatomic, assign) void *gpuiWindow;
@property (nonatomic, strong) CADisplayLink *displayLink;
@end

@implementation ZedraAppDelegate

/// Push the screen scale factor to Rust once at launch.
- (void)pushScreenScale {
    float scale = [UIScreen mainScreen].scale;
    zedra_ios_set_screen_scale(scale);
}

/// Push the current safe area insets (in physical pixels) to Rust.
///
/// UIEdgeInsets are in points; multiply by screen scale to get physical pixels,
/// matching the Android convention expected by PlatformBridge.
/// Must be called after the UIWindow is laid out (deferred on first launch,
/// then on every orientation change and foreground re-entry).
- (void)pushSafeAreaInsets {
    if (!self.gpuiWindow) { return; }
    UIWindow *uiWindow = (__bridge UIWindow *)gpui_ios_get_ui_window(self.gpuiWindow);
    if (!uiWindow) { return; }
    float scale = [UIScreen mainScreen].scale;
    UIEdgeInsets insets = uiWindow.safeAreaInsets;
    zedra_ios_set_safe_area_insets(
        insets.top    * scale,
        insets.bottom * scale,
        insets.left   * scale,
        insets.right  * scale
    );
}

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

    // 6. Push screen scale (known immediately) and safe area insets.
    //    Insets require a layout pass first, so defer one run-loop cycle.
    [self pushScreenScale];
    dispatch_async(dispatch_get_main_queue(), ^{
        [self pushSafeAreaInsets];
    });

    // Re-push insets on orientation change so landscape insets stay correct.
    [[NSNotificationCenter defaultCenter]
        addObserver:self
           selector:@selector(pushSafeAreaInsets)
               name:UIApplicationDidChangeStatusBarOrientationNotification
             object:nil];

    // Track keyboard height for keyboard-avoiding-view (terminal row resize).
    [[NSNotificationCenter defaultCenter]
        addObserver:self
           selector:@selector(keyboardWillShow:)
               name:UIKeyboardWillShowNotification
             object:nil];
    [[NSNotificationCenter defaultCenter]
        addObserver:self
           selector:@selector(keyboardWillHide:)
               name:UIKeyboardWillHideNotification
             object:nil];

    DIAG("launch complete");
    return YES;
}

/// Called when the software keyboard is about to appear or change height.
/// Uses UIKeyboardFrameEndUserInfoKey so we always get the settled keyboard height.
- (void)keyboardWillShow:(NSNotification *)notification {
    NSDictionary *info = [notification userInfo];
    CGRect endFrame = [[info objectForKey:UIKeyboardFrameEndUserInfoKey] CGRectValue];
    float scale = [UIScreen mainScreen].scale;
    unsigned int heightPx = (unsigned int)(endFrame.size.height * scale);
    zedra_ios_set_keyboard_height(heightPx);
}

/// Called when the software keyboard is about to be dismissed.
- (void)keyboardWillHide:(NSNotification *)notification {
    zedra_ios_set_keyboard_height(0);
}

- (void)renderFrame {
    if (self.gpuiWindow) {
        // Check for pending terminal data / callbacks (mirrors Android handle_frame_request).
        // When pending, force a render even if no GPUI views have been explicitly notified,
        // so terminal output and reconnect state changes appear without requiring interaction.
        if (zedra_ios_check_pending_frame()) {
            gpui_ios_request_frame_forced(self.gpuiWindow);
        } else {
            gpui_ios_request_frame(self.gpuiWindow);
        }
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
    // Re-push in case insets changed while backgrounded (e.g. iPad split-screen resize).
    [self pushSafeAreaInsets];
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
