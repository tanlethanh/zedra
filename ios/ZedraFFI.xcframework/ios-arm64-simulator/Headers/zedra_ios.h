#ifndef ZEDRA_IOS_H
#define ZEDRA_IOS_H

#include <stdarg.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdlib.h>

extern void gpui_ios_set_next_embedded_parent(void *parent_view_ptr,
                                              float width_pts,
                                              float height_pts);

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

void zedra_ios_app_will_terminate(void);

bool zedra_ios_system_back(void);

void zedra_ios_native_floating_button_pressed(uint32_t callback_id);

void zedra_ios_dictation_preview_dismiss(uint32_t preview_id);

void zedra_ios_native_edit_menu_result(uint32_t callback_id, int32_t item_index);

void zedra_ios_native_edit_menu_dismiss(uint32_t callback_id);

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

extern bool gpui_ios_is_keyboard_visible(void *window_ptr);

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
 * Returns the native operating system version.
 */
extern const char *ios_get_os_version(void);

/**
 * Returns the native device name for Delta node labels.
 */
extern const char *ios_get_delta_device_name(void);

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
                                  const int32_t *styles,
                                  const char *const *image_names);

extern void ios_present_list_picker(uint32_t callback_id,
                                    const char *title,
                                    const char *message,
                                    int32_t item_count,
                                    const char *const *labels,
                                    const char *const *subtitles,
                                    const char *const *image_names);

/**
 * Present a native edit menu anchored at a window coordinate.
 */
extern void ios_present_native_edit_menu(uint32_t callback_id,
                                         float x_pts,
                                         float y_pts,
                                         int32_t item_count,
                                         const char *const *labels,
                                         const char *const *image_names);

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

extern void ios_dismiss_custom_sheet(void);

/**
 * Open a URL in the system browser via UIApplication.
 */
extern void ios_open_url(const char *url);

/**
 * Open a URL in a native in-app WKWebView.
 */
extern void ios_open_webview(const char *url, const char *title);

/**
 * Trigger a UIKit haptic feedback generator.
 * kind encoding matches HapticFeedback::to_i32().
 */
extern void ios_trigger_haptic(int32_t kind);

/**
 * Play a UI sound effect via AudioToolbox.
 * kind encoding matches SoundEffect::to_i32().
 */
extern void ios_play_sound(int32_t kind);

/**
 * Position or update a native floating icon button.
 */
extern void ios_update_native_floating_button_with_icon(uint32_t callback_id,
                                                        const char *system_image_name,
                                                        const char *accessibility_label,
                                                        float x_pts,
                                                        float y_pts,
                                                        float width_pts,
                                                        float height_pts,
                                                        float icon_size_pts,
                                                        int32_t icon_weight);

/**
 * Hide a native floating icon button.
 */
extern void ios_hide_native_floating_button(uint32_t callback_id);

/**
 * Show or update a native dictation preview overlay.
 */
extern void ios_update_native_dictation_preview(uint32_t preview_id,
                                                const char *text,
                                                float bottom_offset_pts);

/**
 * Hide a native dictation preview overlay.
 */
extern void ios_hide_native_dictation_preview(uint32_t preview_id);

/**
 * Present a native in-app notification banner.
 */
extern void ios_present_native_notification(uint32_t callback_id,
                                            const char *title,
                                            const char *message,
                                            const char *image_name,
                                            int32_t kind,
                                            float duration_secs,
                                            bool auto_close);

/**
 * Start native Google Sign-In for Delta account auth.
 */
extern void ios_start_delta_google_sign_in(uint32_t callback_id);

/**
 * Start native Apple Sign-In for Delta account auth.
 */
extern void ios_start_delta_apple_sign_in(uint32_t callback_id);

/**
 * Request push authorization and return the APNs token.
 */
extern void ios_request_delta_push_token(uint32_t callback_id);

/**
 * Present a native text-input dialog (UIAlertController with UITextField).
 * Result delivered via `zedra_ios_text_input_result` or `zedra_ios_text_input_dismiss`.
 */
extern void ios_present_text_input(uint32_t callback_id,
                                   const char *title,
                                   const char *placeholder,
                                   const char *initial_value);

/**
 * Returns 1 for dark, 0 for light, -1 when unavailable.
 */
extern int32_t ios_system_prefers_dark_theme(void);

/**
 * Apply the app appearance to the native keyboard accessory bar.
 */
extern void ios_set_keyboard_accessory_theme(bool is_dark);

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
 * Called when the user confirms a text-input dialog with the entered value.
 */
void zedra_ios_text_input_result(uint32_t callback_id, const char *value);

/**
 * Called when a text-input dialog is cancelled or dismissed.
 */
void zedra_ios_text_input_dismiss(uint32_t callback_id);

/**
 * Called from the native app delegate when the app enters the background.
 *
 * Drops any unacknowledged native presentation callbacks so captured closures
 * are released and do not accumulate.
 * Wire this to the iOS app delegate's `applicationDidEnterBackground`.
 */
void zedra_ios_app_did_enter_background(void);

void zedra_ios_app_will_enter_foreground(void);

void zedra_ios_native_notification_action(uint32_t callback_id);

void zedra_ios_native_notification_dismiss(uint32_t callback_id);

void zedra_ios_delta_apple_sign_in_result(uint32_t callback_id,
                                          const char *id_token,
                                          const char *email);

void zedra_ios_delta_apple_sign_in_error(uint32_t callback_id, const char *message);

void zedra_ios_delta_google_sign_in_result(uint32_t callback_id,
                                           const char *id_token,
                                           const char *email);

void zedra_ios_delta_google_sign_in_error(uint32_t callback_id, const char *message);

void zedra_ios_delta_push_token_result(uint32_t callback_id,
                                       const char *provider,
                                       const char *token,
                                       const char *environment);

void zedra_ios_delta_push_token_error(uint32_t callback_id, const char *message);

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
