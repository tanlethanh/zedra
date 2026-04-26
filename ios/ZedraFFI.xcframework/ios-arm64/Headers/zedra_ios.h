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

#define BORDER_DEFAULT 2894892

#define BORDER_ACTIVE 5263440

#define BORDER_SUBTLE 1710618

#define ACCENT_GREEN 10011513

#define ACCENT_BLUE 6402031

#define ACCENT_YELLOW 15057019

#define ACCENT_RED 14707829

#define ACCENT_DIM 5263440

#define DRAWER_PADDING 12.0

#define SPACING_SM 8.0

#define SPACING_MD 12.0

#define SPACING_LG 16.0

#define HEADER_HEIGHT 48.0

#define HOME_CARD_WIDTH 300.0

#define HOME_GUIDE_WIDTH 300.0

#define CONNECT_DETAIL_WIDTH 300.0

#define HEADER_BUTTON_SIZE 42.0

#define DRAWER_ICON_ZONE 38.0

#define TERMINAL_LINE_HEIGHT 16.0

#define DRAWER_EDGE_ZONE 56.0

#define DRAWER_VELOCITY_THRESHOLD 12.0

#define DRAWER_BACKDROP_OPACITY 0.4

#define DRAWER_DEFAULT_WIDTH 295.0

#define DRAWER_OPEN_ANIMATION_DURATION_MS 160

#define DRAWER_CLOSE_ANIMATION_DURATION_MS 100

#define FONT_APP_TITLE 28.0

#define FONT_TITLE 20.0

#define FONT_HEADING 13.0

#define FONT_BODY 12.0

#define FONT_DETAIL 12.0

#define ICON_LOGO 20.0

#define ICON_LG 24.0

#define ICON_MD 18.0

#define ICON_SM 16.0

#define ICON_FILE 12.0

#define ICON_FILE_DIR 14.0

#define ICON_STATUS 6.0

#define ICON_TERMINAL 16.0

#define EDITOR_FONT_SIZE 12.0

#define EDITOR_GUTTER_FONT_SIZE 11.0

#define EDITOR_LINE_HEIGHT 15.0

#define EDITOR_GUTTER_WIDTH 36.0

extern void gpui_ios_set_next_embedded_parent(void *parent_view_ptr,
                                              float width_pts,
                                              float height_pts);

extern void *gpui_ios_get_window(void);

extern void gpui_ios_attach_embedded_view(void *window_ptr,
                                          void *parent_view_ptr,
                                          float width_pts,
                                          float height_pts);

extern void gpui_ios_detach_embedded_view(void *window_ptr);

/**
 * Called each frame from main.m before gpui_ios_request_frame.
 *
 * Returns whether the app has explicit pending work that should be surfaced to
 * the frame pump ahead of normal window invalidation. This hook currently does
 * not report any such work, so it returns `false`.
 */
bool zedra_ios_check_pending_frame(void);

void zedra_launch_gpui(void);

void *zedra_ios_mount_custom_sheet_content(void *parent_view_ptr,
                                           float width_pts,
                                           float height_pts);

void zedra_ios_unmount_custom_sheet_content(void);

bool zedra_ios_sheet_content_is_at_top(void);

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

extern void gpui_ios_hide_keyboard(void *window_ptr);

/**
 * Present the AVFoundation QR scanner (defined in QRScanner.swift).
 */
extern void ios_present_qr_scanner(void);

/**
 * Returns the app's Documents directory path (from NSSearchPathForDirectoriesInDomains).
 */
extern const char *ios_get_documents_directory(void);

/**
 * Returns the app's user-facing version string from Info.plist metadata.
 */
extern const char *ios_get_app_version(void);

/**
 * Returns the app's build number string from Info.plist metadata.
 */
extern const char *ios_get_app_build_number(void);

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
 * Present a dismissible native action sheet with dynamic items.
 */
extern void ios_present_selection(uint32_t callback_id,
                                  const char *title,
                                  const char *message,
                                  int32_t button_count,
                                  const char *const *labels,
                                  const int32_t *styles);

/**
 * Present a configurable native custom sheet with a GPUI canvas host.
 */
extern void ios_present_custom_sheet(int32_t detent_count,
                                     const int32_t *detents,
                                     int32_t initial_detent,
                                     bool shows_grabber,
                                     bool expands_on_scroll_edge,
                                     bool edge_attached_in_compact_height,
                                     bool width_follows_preferred_content_size_when_edge_attached,
                                     bool has_corner_radius,
                                     float corner_radius,
                                     bool modal_in_presentation);

/**
 * Open a URL in the system browser via UIApplication.
 */
extern void ios_open_url(const char *url);

/**
 * Trigger a UIKit haptic feedback generator.
 * kind encoding matches HapticFeedback::to_i32().
 */
extern void ios_trigger_haptic(int32_t kind);

/**
 * Present or update a native floating icon button.
 */
extern void ios_present_floating_button(uint32_t callback_id,
                                        const char *system_image_name,
                                        const char *accessibility_label,
                                        float bottom_offset_pts);

/**
 * Dismiss a native floating icon button.
 */
extern void ios_dismiss_floating_button(uint32_t callback_id);

/**
 * Called from the native alert handler after the user taps a button.
 *
 * `callback_id` matches the value passed to `ios_present_alert`.
 * `button_index` is the 0-based index of the tapped button (matches the `buttons` array
 * passed to `platform_bridge::show_alert`).
 */
void zedra_ios_alert_result(uint32_t callback_id, int32_t button_index);

/**
 * Called when an alert is dismissed without choosing a button.
 */
void zedra_ios_alert_dismiss(uint32_t callback_id);

/**
 * Called from the native action sheet handler after the user taps an item.
 */
void zedra_ios_selection_result(uint32_t callback_id, int32_t button_index);

/**
 * Called when an action sheet is dismissed without selecting an item.
 */
void zedra_ios_selection_dismiss(uint32_t callback_id);

/**
 * Called from the native floating button handler after the user taps it.
 */
void zedra_ios_floating_button_result(uint32_t callback_id);

/**
 * Called from the native app delegate when the app enters the background.
 *
 * Drops any unacknowledged alert callbacks so closures captured in them
 * (e.g. `PendingSlot` clones) are released and do not accumulate.
 * Wire this to the iOS app delegate's `applicationDidEnterBackground`.
 */
void zedra_ios_app_did_enter_background(void);

/**
 * Called from the native keyboard accessory bar when a shortcut key button is tapped.
 *
 * `key` is one of: "escape", "tab", "left", "down", "up", "right", "enter", "shift_enter".
 * Maps the name to the corresponding terminal escape sequence and sends it via the active session.
 */
void zedra_ios_send_key_input(const char *key);

/**
 * Called from the native terminal composer to send finalized text to the active terminal.
 */
void zedra_ios_send_terminal_text(const char *text);

/**
 * Called from the native app delegate when the app is opened via a `zedra://` URL.
 */
void zedra_deeplink_received(const char *url);

/**
 * Called from the native QR scanner after a successful QR scan.
 *
 * Routes through the unified deeplink path (same as system URL intents).
 */
void zedra_qr_scanner_result(const char *qr_string);

extern void zedra_nslog(const char *msg);

extern void zedra_log_event(const char *name,
                            const char *const *keys,
                            const char *const *values,
                            int count);

extern void zedra_record_error(const char *message, const char *file, int line);

extern void zedra_record_panic(const char *message, const char *location);

extern void zedra_set_user_id(const char *user_id);

extern void zedra_set_custom_key(const char *key, const char *value);

extern void zedra_set_collection_enabled(int enabled);

#endif  /* ZEDRA_IOS_H */
