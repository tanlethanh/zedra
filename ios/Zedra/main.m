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
extern void gpui_ios_set_keyboard_accessory_view(void* view_ptr);
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
extern void zedra_ios_send_key_input(const char* key);
extern void zedra_launch_gpui(void);
extern void zedra_ios_set_screen_scale(float scale);
extern void zedra_ios_set_safe_area_insets(float top, float bottom, float left, float right);
extern bool zedra_ios_check_pending_frame(void);
extern void zedra_ios_set_keyboard_height(unsigned int height_px);
extern void zedra_ios_alert_result(unsigned int callback_id, int button_index);
extern void zedra_deeplink_received(const char* url);

// Called from Rust to present a native UIAlertController with dynamic buttons.
// `labels` and `styles` are parallel arrays of length `button_count`.
// Style values: 0 = default, 1 = cancel, 2 = destructive.
// Result delivered via zedra_ios_alert_result(callback_id, button_index).
void ios_present_alert(
    unsigned int callback_id,
    const char *title,
    const char *message,
    int button_count,
    const char **labels,
    const int *styles)
{
    // Copy all strings to NSString before the async dispatch (C pointers may be freed).
    NSString *titleStr = (title && title[0]) ? [NSString stringWithUTF8String:title] : nil;
    NSString *messageStr = (message && message[0]) ? [NSString stringWithUTF8String:message] : nil;
    NSMutableArray<NSString *> *labelArr = [NSMutableArray arrayWithCapacity:button_count];
    NSMutableArray<NSNumber *> *styleArr = [NSMutableArray arrayWithCapacity:button_count];
    for (int i = 0; i < button_count; i++) {
        NSString *lbl = (labels && labels[i]) ? [NSString stringWithUTF8String:labels[i]] : @"OK";
        [labelArr addObject:lbl];
        [styleArr addObject:@(styles ? styles[i] : 0)];
    }

    dispatch_async(dispatch_get_main_queue(), ^{
        // Walk connected scenes to find the key window's root view controller.
        UIViewController *vc = nil;
        for (UIWindowScene *scene in [UIApplication sharedApplication].connectedScenes) {
            if (![scene isKindOfClass:[UIWindowScene class]]) continue;
            for (UIWindow *win in ((UIWindowScene *)scene).windows) {
                if (win.isKeyWindow) {
                    vc = win.rootViewController;
                    break;
                }
            }
            if (vc) break;
        }
        // Ascend to the topmost presented controller so the alert appears on top.
        while (vc.presentedViewController) {
            vc = vc.presentedViewController;
        }
        if (!vc) return;

        UIAlertController *alert = [UIAlertController
            alertControllerWithTitle:titleStr
            message:messageStr
            preferredStyle:UIAlertControllerStyleAlert];

        for (int i = 0; i < button_count; i++) {
            UIAlertActionStyle actionStyle;
            switch ([styleArr[i] intValue]) {
                case 1:  actionStyle = UIAlertActionStyleCancel;      break;
                case 2:  actionStyle = UIAlertActionStyleDestructive; break;
                default: actionStyle = UIAlertActionStyleDefault;     break;
            }
            int captured_i = i;
            unsigned int captured_id = callback_id;
            [alert addAction:[UIAlertAction
                actionWithTitle:labelArr[i]
                style:actionStyle
                handler:^(UIAlertAction *action) {
                    zedra_ios_alert_result(captured_id, captured_i);
                }]];
        }

        [vc presentViewController:alert animated:YES completion:nil];
    });
}

// Returns the app's Documents directory as a C string (static buffer).
// Called from Rust via FFI to determine where to persist workspace data.
const char* ios_get_documents_directory(void) {
    static char buf[1024];
    NSArray *paths = NSSearchPathForDirectoriesInDomains(
        NSDocumentDirectory, NSUserDomainMask, YES);
    if (paths.count == 0) return NULL;
    const char *cstr = [paths[0] UTF8String];
    if (!cstr) return NULL;
    strlcpy(buf, cstr, sizeof(buf));
    return buf;
}

/// Key names indexed by button tag (matches order in setupKeyboardAccessoryView).
static const char *kAccessoryKeyNames[] = {"escape", "tab", "left", "down", "up", "right", "enter"};

@interface ZedraAppDelegate : UIResponder <UIApplicationDelegate>
@property (nonatomic, assign) void *gpuiApp;
@property (nonatomic, assign) void *gpuiWindow;
@property (nonatomic, strong) CADisplayLink *displayLink;
/// Toolbar shown above the software keyboard with terminal shortcut keys.
@property (nonatomic, strong) UIToolbar *keyboardAccessoryBar;
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

        // Set up the keyboard accessory bar (shortcut keys above the software keyboard).
        [self setupKeyboardAccessoryView];

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

/// Create a UIToolbar with 6 shortcut keys and register it as the keyboard accessory view.
///
/// The toolbar is attached above the software keyboard whenever the GPUI Metal view
/// becomes first responder. Button taps are routed to Rust via zedra_ios_send_key_input.
- (void)setupKeyboardAccessoryView {
    CGFloat width = [UIScreen mainScreen].bounds.size.width;
    UIToolbar *toolbar = [[UIToolbar alloc] initWithFrame:CGRectMake(0, 0, width, 44)];

    // Use the system chrome material — the same background the iOS keyboard uses.
    // configureWithDefaultBackground() applies a UIBlurEffect that automatically
    // adapts to light/dark mode, producing a seamless join with the keyboard.
    UIToolbarAppearance *appearance = [[UIToolbarAppearance alloc] init];
    [appearance configureWithDefaultBackground];
    toolbar.standardAppearance = appearance;
    toolbar.scrollEdgeAppearance = appearance;
    toolbar.compactAppearance = appearance;

    // Use the primary label color so buttons read well on both light and dark keyboards.
    toolbar.tintColor = [UIColor labelColor];

    NSArray<NSString *> *labels = @[@"Esc", @"Tab", @"←", @"↓", @"↑", @"→", @"⏎"];

    UIBarButtonItem *flex = [[UIBarButtonItem alloc]
        initWithBarButtonSystemItem:UIBarButtonSystemItemFlexibleSpace
        target:nil action:nil];

    NSMutableArray<UIBarButtonItem *> *items = [NSMutableArray array];
    for (NSInteger i = 0; i < (NSInteger)labels.count; i++) {
        if (i > 0) [items addObject:flex];
        UIBarButtonItem *btn = [[UIBarButtonItem alloc]
            initWithTitle:labels[i]
            style:UIBarButtonItemStylePlain
            target:self
            action:@selector(keyboardShortcutTapped:)];
        btn.tag = i;
        [items addObject:btn];
    }

    toolbar.items = items;
    self.keyboardAccessoryBar = toolbar;
    gpui_ios_set_keyboard_accessory_view((__bridge void *)toolbar);
}

/// Handles taps on keyboard shortcut buttons; sends the corresponding escape sequence.
- (void)keyboardShortcutTapped:(UIBarButtonItem *)sender {
    NSInteger idx = sender.tag;
    if (idx >= 0 && idx < 7) {
        zedra_ios_send_key_input(kAccessoryKeyNames[idx]);
    }
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

- (BOOL)application:(UIApplication *)app
            openURL:(NSURL *)url
            options:(NSDictionary<UIApplicationOpenURLOptionsKey, id> *)options {
    NSString *urlString = [url absoluteString];
    if (urlString) {
        zedra_deeplink_received([urlString UTF8String]);
    }
    return YES;
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
