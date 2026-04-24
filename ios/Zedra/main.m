#import <UIKit/UIKit.h>
#import <string.h>
#import <unistd.h>

int main(int argc, char * argv[]) {
    @autoreleasepool {
        Class appDelegateClass = NSClassFromString(@"ZedraAppDelegate");
        return UIApplicationMain(argc, argv, nil,
                                 NSStringFromClass(appDelegateClass));
    }
}
