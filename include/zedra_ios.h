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

#define BORDER_DEFAULT 5263440

#define BORDER_SUBTLE 1710618

#define ACCENT_GREEN 10011513

#define ACCENT_BLUE 6402031

#define ACCENT_YELLOW 15057019

#define ACCENT_RED 14707829

#define FONT_TITLE 22.0

#define FONT_HEADING 14.0

#define FONT_BODY 12.0

#define FONT_DETAIL 10.0

#define ICON_NAV 18.0

#define ICON_HEADER 20.0

#define ICON_FILE 12.0

#define ICON_FILE_DIR 14.0

#define ICON_STATUS 6.0

#define EDITOR_FONT_SIZE 12.0

#define EDITOR_GUTTER_FONT_SIZE 11.0

#define EDITOR_LINE_HEIGHT 15.0

#define EDITOR_GUTTER_WIDTH 36.0

extern void *gpui_ios_get_window(void);

extern void gpui_ios_show_keyboard(void *window_ptr);

extern void gpui_ios_hide_keyboard(void *window_ptr);

void zedra_launch_gpui(void);

#endif  /* ZEDRA_IOS_H */
