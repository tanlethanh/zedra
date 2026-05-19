# Rust reaches these Android methods by exact JNI class/name/signature.
-keep class dev.zedra.app.MainActivity {
    public static void showKeyboard();
    public static void hideKeyboard();
    public static void launchQrScanner();
    public static void openUrl(java.lang.String);
    public static void showAlert(int, java.lang.String, java.lang.String, java.lang.String[], int[]);
    public static void showSelection(int, java.lang.String, java.lang.String, java.lang.String[], int[]);
    public static void showTextInput(int, java.lang.String, java.lang.String, java.lang.String);
    public static void presentCustomSheet(int[], int, boolean, boolean, boolean, float);
    public static void updateNativeFloatingButton(int, java.lang.String, java.lang.String, float, float, float, float, float, int);
    public static void hideNativeFloatingButton(int);
    public static void updateNativeDictationPreview(int, java.lang.String, float);
    public static void hideNativeDictationPreview(int);
    public static void showNativeNotification(int, java.lang.String, java.lang.String, java.lang.String, int, float, boolean);
    public static void triggerHaptic(int);
}

# JNI native methods are bound by generated Rust export names, so class and
# member names must survive release obfuscation.
-keepclasseswithmembernames class dev.zedra.app.** {
    native <methods>;
}

-keepclasseswithmembernames class dev.zed.gpui.** {
    native <methods>;
}
