#ifndef ZEDRA_IOS_H
#define ZEDRA_IOS_H

#include <stdarg.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdlib.h>

#define BG_PRIMARY 920588

#define BG_CARD 1250067

#define BG_OVERLAY 1250067

#define BG_SURFACE 920588

#define TEXT_PRIMARY 16777215

#define TEXT_SECONDARY 13290186

#define TEXT_MUTED 5263440

#define BORDER_ACTIVE TEXT_PRIMARY

#define BORDER_DEFAULT 5263440

#define BORDER_SUBTLE 1710618

#define ACCENT_GREEN 10011513

#define ACCENT_BLUE 6402031

#define ACCENT_YELLOW 15057019

#define ACCENT_RED 14707829

#define DRAWER_PADDING 12.0

#define FONT_TITLE 22.0

#define FONT_HEADING 13.0

#define FONT_BODY 12.0

#define FONT_DETAIL 12.0

#define ICON_NAV 18.0

#define ICON_HEADER 18.0

#define ICON_FILE 12.0

#define ICON_FILE_DIR 14.0

#define ICON_STATUS 6.0

#define EDITOR_FONT_SIZE 12.0

#define EDITOR_GUTTER_FONT_SIZE 11.0

#define EDITOR_LINE_HEIGHT 15.0

#define EDITOR_GUTTER_WIDTH 36.0

/**
 * Called from Obj-C whenever the screen scale is known (once, at launch).
 *
 * Pass `[UIScreen mainScreen].scale`.
 */
void zedra_ios_set_screen_scale(float scale);

/**
 * Called from Obj-C when the software keyboard is about to appear or change height.
 *
 * `height_px` is `endFrame.size.height × UIScreen.scale` (physical pixels).
 * Call with 0 when the keyboard is dismissed.
 */
void zedra_ios_set_keyboard_height(uint32_t height_px);

/**
 * Called from Obj-C with the current safe area insets in physical pixels
 * (UIEdgeInsets × UIScreen.scale). Re-called on orientation change.
 *
 * `left` and `right` are stored for future use (landscape support).
 */
void zedra_ios_set_safe_area_insets(float top, float bottom, float _left, float _right);

extern void *gpui_ios_get_window(void);

extern bool gpui_ios_is_keyboard_visible(void *window_ptr);

extern void gpui_ios_show_keyboard(void *window_ptr);

extern void gpui_ios_hide_keyboard(void *window_ptr);

/**
 * Present the AVFoundation QR scanner (defined in ZedraQRScanner.m).
 */
extern void ios_present_qr_scanner(void);

/**
 * Returns the app's Documents directory path (from NSSearchPathForDirectoriesInDomains).
 */
extern const char *ios_get_documents_directory(void);

/**
 * Present a native UIAlertController with dynamic buttons.
 * `labels` and `styles` are parallel arrays of length `button_count`.
 * Style values: 0 = default, 1 = cancel, 2 = destructive.
 * Result delivered via `zedra_ios_alert_result(callback_id, button_index)`.
 */
extern void ios_present_alert(uint32_t callback_id,
                              const char *title,
                              const char *message,
                              int32_t button_count,
                              const char *const *labels,
                              const int32_t *styles);

/**
 * Called from the UIAlertController handler in main.m after the user taps a button.
 *
 * `callback_id` matches the value passed to `ios_present_alert`.
 * `button_index` is the 0-based index of the tapped button (matches the `buttons` array
 * passed to `platform_bridge::show_alert`).
 */
void zedra_ios_alert_result(uint32_t callback_id, int32_t button_index);

/**
 * Called from the native keyboard accessory bar when a shortcut key button is tapped.
 *
 * `key` is one of: "escape", "tab", "left", "down", "up", "right", "enter".
 * Maps the name to the corresponding terminal escape sequence and sends it via the active session.
 */
void zedra_ios_send_key_input(const char *key);

/**
 * Called from main.m when the app is opened via a zedra:// URL.
 */
void zedra_deeplink_received(const char *url);

/**
 * Called from ZedraQRScanner.m after a successful QR scan.
 *
 * Routes through the unified deeplink path (same as system URL intents).
 */
void zedra_qr_scanner_result(const char *qr_string);

/**
 * Called each frame from main.m before gpui_ios_request_frame.
 *
 * Drains main-thread callbacks and checks whether terminal data is pending.
 * Returns `true` when a forced render is needed (mirrors Android's
 * `check_and_clear_terminal_data` + `drain_callbacks` in `handle_frame_request`).
 */
bool zedra_ios_check_pending_frame(void);

void zedra_launch_gpui(void);

extern void zedra_nslog(const char *msg);

extern void zedra_log_event(const char *name,
                            const char *const *keys,
                            const char *const *values,
                            int count);

extern void zedra_record_error(const char *message, const char *file, int line);

extern void zedra_record_panic(const char *message, const char *location);

extern void zedra_set_user_id(const char *user_id);

extern void zedra_set_custom_key(const char *key, const char *value);

#endif  /* ZEDRA_IOS_H */
