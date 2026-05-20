# App and GPUI Android Kotlin packages are small JNI boundary packages. Rust
# reaches them by exact class/name/signature, while native presentations and
# framework lifecycle callbacks cross back through generated Kotlin methods.
# Keep both packages stable in release and let R8 shrink dependencies instead.
-keep class dev.zedra.app.** { *; }
-keep class dev.zed.gpui.** { *; }
