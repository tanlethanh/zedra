// Weak stub for ios_present_qr_scanner.
//
// Purpose: allows the Rust cdylib to link without errors when targeting iOS.
// The real implementation is in ios/Zedra/ZedraQRScanner.m and is linked by Xcode.
// Because this is __attribute__((weak)), the linker always prefers the strong
// ObjC definition at Xcode link time.
__attribute__((weak)) void ios_present_qr_scanner(void) {}
