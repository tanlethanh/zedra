package dev.zedra.app;

import androidx.appcompat.app.AppCompatActivity;

import android.os.Build;
import android.os.Bundle;
import android.view.View;
import android.view.Window;
import android.widget.TextView;

public class MainActivity extends AppCompatActivity {
    static {
        System.loadLibrary("zedra");
    }

    // Native methods
    public static native void initRust();
    public static native String rustGreeting(String input);
    public static native void rustOnResume();
    public static native void rustOnPause();

    @Override
    protected void onCreate(Bundle savedInstanceState) {
        super.onCreate(savedInstanceState);
        setContentView(R.layout.activity_main);

        Window window = getWindow();

        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
            // Edge to edge
            window.setDecorFitsSystemWindows(false);
        } else {
            View decorView = window.getDecorView();
            decorView.setSystemUiVisibility(
                View.SYSTEM_UI_FLAG_LAYOUT_STABLE
                    | View.SYSTEM_UI_FLAG_LAYOUT_FULLSCREEN
                    | View.SYSTEM_UI_FLAG_LAYOUT_HIDE_NAVIGATION
            );
        }

        // Initialize Rust
        initRust();

        // Test the Rust function
        TextView textView = findViewById(R.id.text_view);
        String greeting = rustGreeting("Android");
        textView.setText(greeting);
    }

    @Override
    protected void onResume() {
        super.onResume();
        rustOnResume();
    }

    @Override
    protected void onPause() {
        super.onPause();
        rustOnPause();
    }
}
