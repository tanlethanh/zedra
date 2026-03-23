// Zedra iOS — QR Code Scanner
//
// Presents a full-screen AVFoundation camera view that scans QR codes.
// On successful scan, calls zedra_qr_scanner_result() with the raw string
// (a base64-url encoded iroh::EndpointAddr) so Rust can decode it.
//
// Entry point:   ios_present_qr_scanner()  — called from Rust bridge
// Rust callback: zedra_qr_scanner_result() — defined in Rust, declared below

#import <UIKit/UIKit.h>
#import <AVFoundation/AVFoundation.h>

// Rust FFI callback (defined in crates/zedra/src/ios/bridge.rs)
extern void zedra_qr_scanner_result(const char *qr_string);

// ─────────────────────────────────────────────
// ZedraQRScannerVC
// ─────────────────────────────────────────────

@interface ZedraQRScannerVC : UIViewController <AVCaptureMetadataOutputObjectsDelegate>
@property (nonatomic, strong) AVCaptureSession *session;
@property (nonatomic, strong) AVCaptureVideoPreviewLayer *previewLayer;
@end

@implementation ZedraQRScannerVC

- (void)viewDidLoad {
    [super viewDidLoad];
    self.view.backgroundColor = [UIColor blackColor];

    // Cancel button — top-right, above safe area
    UIButton *cancelBtn = [UIButton buttonWithType:UIButtonTypeSystem];
    [cancelBtn setTitle:@"Cancel" forState:UIControlStateNormal];
    [cancelBtn setTitleColor:[UIColor whiteColor] forState:UIControlStateNormal];
    cancelBtn.titleLabel.font = [UIFont systemFontOfSize:17 weight:UIFontWeightSemibold];
    cancelBtn.translatesAutoresizingMaskIntoConstraints = NO;
    [cancelBtn addTarget:self action:@selector(cancelTapped) forControlEvents:UIControlEventTouchUpInside];
    [self.view addSubview:cancelBtn];

    [NSLayoutConstraint activateConstraints:@[
        [cancelBtn.topAnchor constraintEqualToAnchor:self.view.safeAreaLayoutGuide.topAnchor constant:12],
        [cancelBtn.trailingAnchor constraintEqualToAnchor:self.view.trailingAnchor constant:-20],
    ]];

    [self requestCameraAndStart];
}

- (void)requestCameraAndStart {
    AVAuthorizationStatus status = [AVCaptureDevice authorizationStatusForMediaType:AVMediaTypeVideo];
    if (status == AVAuthorizationStatusAuthorized) {
        [self setupCamera];
    } else if (status == AVAuthorizationStatusNotDetermined) {
        [AVCaptureDevice requestAccessForMediaType:AVMediaTypeVideo completionHandler:^(BOOL granted) {
            dispatch_async(dispatch_get_main_queue(), ^{
                if (granted) [self setupCamera];
                else [self showPermissionDenied];
            });
        }];
    } else {
        [self showPermissionDenied];
    }
}

- (void)setupCamera {
    AVCaptureDevice *device = [AVCaptureDevice defaultDeviceWithMediaType:AVMediaTypeVideo];
    if (!device) {
        [self cancelTapped];
        return;
    }

    NSError *error = nil;
    AVCaptureDeviceInput *input = [AVCaptureDeviceInput deviceInputWithDevice:device error:&error];
    if (!input) {
        [self cancelTapped];
        return;
    }

    self.session = [[AVCaptureSession alloc] init];
    [self.session addInput:input];

    AVCaptureMetadataOutput *output = [[AVCaptureMetadataOutput alloc] init];
    [self.session addOutput:output];
    [output setMetadataObjectsDelegate:self queue:dispatch_get_main_queue()];
    output.metadataObjectTypes = @[AVMetadataObjectTypeQRCode];

    self.previewLayer = [AVCaptureVideoPreviewLayer layerWithSession:self.session];
    self.previewLayer.frame = self.view.bounds;
    self.previewLayer.videoGravity = AVLayerVideoGravityResizeAspectFill;
    [self.view.layer insertSublayer:self.previewLayer atIndex:0];

    [self updatePreviewOrientation];

    dispatch_async(dispatch_get_global_queue(QOS_CLASS_USER_INTERACTIVE, 0), ^{
        [self.session startRunning];
    });
}

// Sync the preview layer's video orientation with the current interface orientation.
- (void)updatePreviewOrientation {
    AVCaptureConnection *conn = self.previewLayer.connection;
    if (!conn) return;

    UIInterfaceOrientation ifOrientation = UIInterfaceOrientationUnknown;
    for (UIWindowScene *scene in UIApplication.sharedApplication.connectedScenes) {
        if ([scene isKindOfClass:[UIWindowScene class]]) {
            ifOrientation = ((UIWindowScene *)scene).effectiveGeometry.interfaceOrientation;
            break;
        }
    }

#pragma clang diagnostic push
#pragma clang diagnostic ignored "-Wdeprecated-declarations"
    AVCaptureVideoOrientation videoOrientation;
    switch (ifOrientation) {
        case UIInterfaceOrientationLandscapeLeft:
            videoOrientation = AVCaptureVideoOrientationLandscapeLeft;
            break;
        case UIInterfaceOrientationLandscapeRight:
            videoOrientation = AVCaptureVideoOrientationLandscapeRight;
            break;
        case UIInterfaceOrientationPortraitUpsideDown:
            videoOrientation = AVCaptureVideoOrientationPortraitUpsideDown;
            break;
        default:
            videoOrientation = AVCaptureVideoOrientationPortrait;
            break;
    }
    if (conn.isVideoOrientationSupported) {
        conn.videoOrientation = videoOrientation;
    }
#pragma clang diagnostic pop
}

- (void)viewWillTransitionToSize:(CGSize)size
       withTransitionCoordinator:(id<UIViewControllerTransitionCoordinator>)coordinator {
    [super viewWillTransitionToSize:size withTransitionCoordinator:coordinator];
    [coordinator animateAlongsideTransition:nil completion:^(id<UIViewControllerTransitionCoordinatorContext> ctx) {
        [self updatePreviewOrientation];
    }];
}

- (void)viewDidLayoutSubviews {
    [super viewDidLayoutSubviews];
    self.previewLayer.frame = self.view.bounds;
}

// AVCaptureMetadataOutputObjectsDelegate
- (void)captureOutput:(AVCaptureOutput *)output
    didOutputMetadataObjects:(NSArray<__kindof AVMetadataObject *> *)metadataObjects
           fromConnection:(AVCaptureConnection *)connection {
    for (AVMetadataObject *obj in metadataObjects) {
        if (![obj isKindOfClass:[AVMetadataMachineReadableCodeObject class]]) continue;
        AVMetadataMachineReadableCodeObject *code = (AVMetadataMachineReadableCodeObject *)obj;
        if (!code.stringValue) continue;

        [self.session stopRunning];
        zedra_qr_scanner_result([code.stringValue UTF8String]);
        [self dismissViewControllerAnimated:YES completion:nil];
        return;
    }
}

- (void)cancelTapped {
    if (self.session.isRunning) [self.session stopRunning];
    [self dismissViewControllerAnimated:YES completion:nil];
}

- (void)showPermissionDenied {
    UIAlertController *alert = [UIAlertController
        alertControllerWithTitle:@"Camera Access Required"
        message:@"Please enable camera access in Settings to scan QR codes."
        preferredStyle:UIAlertControllerStyleAlert];
    [alert addAction:[UIAlertAction
        actionWithTitle:@"OK"
        style:UIAlertActionStyleDefault
        handler:^(UIAlertAction *a) { [self cancelTapped]; }]];
    [self presentViewController:alert animated:YES completion:nil];
}

@end

// ─────────────────────────────────────────────
// C entry point — called from Rust via FFI
// ─────────────────────────────────────────────

void ios_present_qr_scanner(void) {
    dispatch_async(dispatch_get_main_queue(), ^{
        // Find the key window's top-most view controller
        UIWindow *keyWindow = nil;
        for (UIWindowScene *scene in UIApplication.sharedApplication.connectedScenes) {
            if (![scene isKindOfClass:[UIWindowScene class]]) continue;
            for (UIWindow *window in ((UIWindowScene *)scene).windows) {
                if (window.isKeyWindow) { keyWindow = window; break; }
            }
            if (keyWindow) break;
        }

        UIViewController *presenter = keyWindow.rootViewController;
        while (presenter.presentedViewController) {
            presenter = presenter.presentedViewController;
        }

        if (!presenter) return;

        ZedraQRScannerVC *vc = [[ZedraQRScannerVC alloc] init];
        vc.modalPresentationStyle = UIModalPresentationFullScreen;
        [presenter presentViewController:vc animated:YES completion:nil];
    });
}

void ios_open_url(const char *url) {
    if (!url) return;
    NSString *urlStr = [NSString stringWithUTF8String:url];
    NSURL *nsUrl = [NSURL URLWithString:urlStr];
    if (!nsUrl) return;
    dispatch_async(dispatch_get_main_queue(), ^{
        [[UIApplication sharedApplication] openURL:nsUrl options:@{} completionHandler:nil];
    });
}

void ios_copy_to_clipboard(const char *text) {
    if (!text) return;
    NSString *str = [NSString stringWithUTF8String:text];
    dispatch_async(dispatch_get_main_queue(), ^{
        [UIPasteboard generalPasteboard].string = str;
    });
}
