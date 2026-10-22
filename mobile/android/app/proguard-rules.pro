# JNA and the generated bindings are reached through reflection and native callbacks.
-keep class com.sun.jna.** { *; }
-keep class * implements com.sun.jna.** { *; }
-keep class uniffi.hush_ffi.** { *; }
-dontwarn java.awt.**
-dontwarn com.sun.jna.**

# Nothing the app says reaches the system log in release builds.
-assumenosideeffects class android.util.Log {
    public static int v(...);
    public static int d(...);
    public static int i(...);
    public static int w(...);
    public static int e(...);
}
