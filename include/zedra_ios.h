#ifndef ZEDRA_IOS_H
#define ZEDRA_IOS_H

#include <stdarg.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdlib.h>

/**
 * Create the GPUI Application and open the main window.
 *
 * Lifecycle (called from Obj-C app delegate):
 *   1. gpui_ios_initialize()         — set up GPUI FFI state
 *   2. zedra_launch_gpui()           — create Application, register window callback
 *   3. gpui_ios_did_finish_launching — invoke callback → opens window
 *   4. gpui_ios_get_window()         — get window pointer for CADisplayLink
 *   5. gpui_ios_request_frame()      — called each frame by CADisplayLink
 */
void zedra_launch_gpui(void);

/**
 * Initialize the Zedra Rust backend.
 *
 * Must be called once at app launch (e.g., in SwiftUI App.init()).
 * Sets up logging via oslog and initializes the session runtime.
 */
void zedra_init(void);

/**
 * Initialize with screen dimensions and scale factor.
 *
 * Call after zedra_init() with the device's screen info:
 *   - width/height: screen size in points
 *   - scale: UIScreen.main.scale (e.g. 2.0 or 3.0)
 */
void zedra_init_screen(float width, float height, float scale);

/**
 * Process all pending commands and tick the frame.
 *
 * Must be called from the main thread (e.g., via CADisplayLink callback).
 */
void zedra_process_frame(void);

/**
 * Connect to a zedra-host daemon at the given host:port.
 *
 * The connection is asynchronous. Poll zedra_get_connection_status() to check progress.
 */
void zedra_connect(const char *host, uint16_t port);

/**
 * Disconnect the active session.
 */
void zedra_disconnect(void);

/**
 * Pair via QR code data (zedra:// URI).
 */
void zedra_pair_via_qr(const char *data);

/**
 * Send text input to the active terminal session.
 */
void zedra_send_input(const char *text);

/**
 * Send a special key event (backspace, enter, tab, escape, arrow keys).
 */
void zedra_send_key(const char *key_name);

/**
 * Get pending terminal output since last call.
 *
 * Returns a C string that the caller must free with zedra_free_string().
 * Returns NULL if no output is available.
 */
char *zedra_get_terminal_output(void);

/**
 * Get the current connection status.
 *
 * Returns: 0=disconnected, 1=connecting, 2=connected, 3=error
 */
int32_t zedra_get_connection_status(void);

/**
 * Get the connection error message (if status == 3).
 *
 * Returns a C string that the caller must free with zedra_free_string().
 * Returns NULL if no error.
 */
char *zedra_get_connection_error(void);

/**
 * Get the current transport info string (e.g. "LAN · 12ms").
 *
 * Returns a C string that the caller must free with zedra_free_string().
 * Returns NULL if no transport info available.
 */
char *zedra_get_transport_info(void);

/**
 * Notify that the app has entered foreground.
 */
void zedra_on_resume(void);

/**
 * Notify that the app has entered background.
 */
void zedra_on_pause(void);

/**
 * Free a string previously returned by Rust.
 *
 * Must be called for every non-NULL string returned by zedra_get_terminal_output(),
 * zedra_get_connection_error(), zedra_get_transport_info(), etc.
 */
void zedra_free_string(char *ptr);

/**
 * Forward a touch event to the Rust backend.
 *
 * action: 0=began, 1=ended, 2=moved, 3=cancelled
 * x, y: position in points
 */
void zedra_touch_event(int32_t action, float x, float y);

/**
 * Notify that the view has been resized.
 */
void zedra_view_resized(float width, float height);

#endif  /* ZEDRA_IOS_H */
