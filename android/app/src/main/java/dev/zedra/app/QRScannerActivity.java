package dev.zedra.app;

import android.Manifest;
import android.content.pm.PackageManager;
import android.graphics.Canvas;
import android.graphics.Color;
import android.graphics.CornerPathEffect;
import android.graphics.Paint;
import android.graphics.Path;
import android.os.Build;
import android.os.Bundle;
import android.util.Log;
import android.util.Size;
import android.view.Gravity;
import android.view.View;
import android.view.Window;
import android.view.WindowManager;
import android.widget.FrameLayout;
import android.widget.TextView;
import android.widget.Toast;

import androidx.annotation.NonNull;
import androidx.appcompat.app.AppCompatActivity;
import androidx.camera.core.CameraSelector;
import androidx.camera.core.ImageAnalysis;
import androidx.camera.core.ImageProxy;
import androidx.camera.core.Preview;
import androidx.camera.lifecycle.ProcessCameraProvider;
import androidx.camera.view.PreviewView;
import androidx.core.app.ActivityCompat;
import androidx.core.content.ContextCompat;

import com.google.common.util.concurrent.ListenableFuture;
import com.google.mlkit.vision.barcode.BarcodeScanner;
import com.google.mlkit.vision.barcode.BarcodeScannerOptions;
import com.google.mlkit.vision.barcode.BarcodeScanning;
import com.google.mlkit.vision.barcode.common.Barcode;
import com.google.mlkit.vision.common.InputImage;

import java.util.concurrent.ExecutionException;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;

/**
 * Activity for scanning QR codes for device pairing.
 * Uses CameraX for camera access and ML Kit for barcode detection.
 */
public class QRScannerActivity extends AppCompatActivity {
    private static final String TAG = "QRScanner";
    private static final int CAMERA_PERMISSION_CODE = 100;
    private static final String ZEDRA_URI_PREFIX = "zedra://";

    private PreviewView previewView;
    private ExecutorService cameraExecutor;
    private BarcodeScanner barcodeScanner;
    private boolean scanComplete = false;

    // Native method to send QR data to Rust
    private static native void nativeOnQrCodeScanned(String data);

    @Override
    protected void onCreate(Bundle savedInstanceState) {
        super.onCreate(savedInstanceState);

        // Transparent status bar — camera preview extends behind it
        Window window = getWindow();
        window.addFlags(WindowManager.LayoutParams.FLAG_DRAWS_SYSTEM_BAR_BACKGROUNDS);
        window.setStatusBarColor(Color.TRANSPARENT);
        window.setNavigationBarColor(Color.TRANSPARENT);
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
            window.setDecorFitsSystemWindows(false);
        } else {
            window.getDecorView().setSystemUiVisibility(
                    View.SYSTEM_UI_FLAG_LAYOUT_STABLE
                            | View.SYSTEM_UI_FLAG_LAYOUT_FULLSCREEN
                            | View.SYSTEM_UI_FLAG_LAYOUT_HIDE_NAVIGATION);
        }

        // Build layout programmatically
        FrameLayout root = new FrameLayout(this);
        root.setBackgroundColor(0xFF000000);

        // Full-screen camera preview
        previewView = new PreviewView(this);
        root.addView(previewView, new FrameLayout.LayoutParams(
                FrameLayout.LayoutParams.MATCH_PARENT,
                FrameLayout.LayoutParams.MATCH_PARENT));

        // Viewfinder overlay: semi-transparent scrim with a clear square cutout + corner brackets
        root.addView(new ViewfinderOverlay(this), new FrameLayout.LayoutParams(
                FrameLayout.LayoutParams.MATCH_PARENT,
                FrameLayout.LayoutParams.MATCH_PARENT));

        // Hint label below the viewfinder
        TextView hint = new TextView(this);
        hint.setText("Point camera at a Zedra QR code");
        hint.setTextColor(0xDDFFFFFF);
        hint.setTextSize(15);
        FrameLayout.LayoutParams hintParams = new FrameLayout.LayoutParams(
                FrameLayout.LayoutParams.WRAP_CONTENT,
                FrameLayout.LayoutParams.WRAP_CONTENT,
                Gravity.BOTTOM | Gravity.CENTER_HORIZONTAL);
        hintParams.bottomMargin = 160;
        root.addView(hint, hintParams);

        setContentView(root);

        // Initialize barcode scanner with QR code filter
        BarcodeScannerOptions options = new BarcodeScannerOptions.Builder()
                .setBarcodeFormats(Barcode.FORMAT_QR_CODE)
                .build();
        barcodeScanner = BarcodeScanning.getClient(options);

        cameraExecutor = Executors.newSingleThreadExecutor();

        // Check camera permission
        if (ContextCompat.checkSelfPermission(this, Manifest.permission.CAMERA)
                == PackageManager.PERMISSION_GRANTED) {
            startCamera();
        } else {
            ActivityCompat.requestPermissions(this,
                    new String[]{Manifest.permission.CAMERA},
                    CAMERA_PERMISSION_CODE);
        }
    }

    @Override
    public void onRequestPermissionsResult(int requestCode, @NonNull String[] permissions,
                                           @NonNull int[] grantResults) {
        if (requestCode == CAMERA_PERMISSION_CODE) {
            if (grantResults.length > 0 && grantResults[0] == PackageManager.PERMISSION_GRANTED) {
                startCamera();
            } else {
                Toast.makeText(this, "Camera permission required for QR scanning",
                        Toast.LENGTH_LONG).show();
                finish();
            }
        }
    }

    private void startCamera() {
        ListenableFuture<ProcessCameraProvider> cameraProviderFuture =
                ProcessCameraProvider.getInstance(this);

        cameraProviderFuture.addListener(() -> {
            try {
                ProcessCameraProvider cameraProvider = cameraProviderFuture.get();

                // Preview use case — connect to the PreviewView surface
                Preview preview = new Preview.Builder().build();
                preview.setSurfaceProvider(previewView.getSurfaceProvider());

                // Image analysis for QR detection
                ImageAnalysis imageAnalysis = new ImageAnalysis.Builder()
                        .setTargetResolution(new Size(1280, 720))
                        .setBackpressureStrategy(ImageAnalysis.STRATEGY_KEEP_ONLY_LATEST)
                        .build();

                imageAnalysis.setAnalyzer(cameraExecutor, this::analyzeImage);

                // Select back camera
                CameraSelector cameraSelector = CameraSelector.DEFAULT_BACK_CAMERA;

                // Unbind all use cases and rebind
                cameraProvider.unbindAll();
                cameraProvider.bindToLifecycle(
                        this,
                        cameraSelector,
                        preview,
                        imageAnalysis
                );

            } catch (ExecutionException | InterruptedException e) {
                Log.e(TAG, "Failed to start camera", e);
            }
        }, ContextCompat.getMainExecutor(this));
    }

    @SuppressWarnings("UnsafeOptInUsageError")
    private void analyzeImage(ImageProxy imageProxy) {
        if (scanComplete) {
            imageProxy.close();
            return;
        }

        android.media.Image mediaImage = imageProxy.getImage();
        if (mediaImage == null) {
            imageProxy.close();
            return;
        }

        InputImage image = InputImage.fromMediaImage(
                mediaImage, imageProxy.getImageInfo().getRotationDegrees());

        barcodeScanner.process(image)
                .addOnSuccessListener(barcodes -> {
                    for (Barcode barcode : barcodes) {
                        String rawValue = barcode.getRawValue();
                        if (rawValue != null && rawValue.startsWith(ZEDRA_URI_PREFIX)) {
                            Log.i(TAG, "Zedra QR code detected");
                            scanComplete = true;

                            // Send to Rust via JNI
                            nativeOnQrCodeScanned(rawValue);

                            // Close scanner
                            runOnUiThread(this::finish);
                            break;
                        }
                    }
                })
                .addOnFailureListener(e -> {
                    Log.e(TAG, "Barcode scanning failed", e);
                })
                .addOnCompleteListener(task -> {
                    imageProxy.close();
                });
    }

    @Override
    protected void onDestroy() {
        super.onDestroy();
        if (cameraExecutor != null) {
            cameraExecutor.shutdown();
        }
        if (barcodeScanner != null) {
            barcodeScanner.close();
        }
    }

    /**
     * Custom overlay that draws a dark scrim with a transparent square cutout
     * and rounded corner brackets as the viewfinder guide.
     */
    private static class ViewfinderOverlay extends View {
        private final Paint scrimPaint = new Paint();
        private final Paint cornerPaint = new Paint(Paint.ANTI_ALIAS_FLAG);

        public ViewfinderOverlay(android.content.Context context) {
            super(context);
            scrimPaint.setColor(0x88000000); // 53% black
            scrimPaint.setStyle(Paint.Style.FILL);

            cornerPaint.setColor(0xFFFFFFFF);
            cornerPaint.setStyle(Paint.Style.STROKE);
            cornerPaint.setStrokeWidth(4 * context.getResources().getDisplayMetrics().density);
            cornerPaint.setStrokeCap(Paint.Cap.ROUND);
            cornerPaint.setPathEffect(new CornerPathEffect(
                    8 * context.getResources().getDisplayMetrics().density));
        }

        @Override
        protected void onDraw(Canvas canvas) {
            int w = getWidth();
            int h = getHeight();
            float dp = getResources().getDisplayMetrics().density;

            // Square size = 65% of the narrower dimension
            float side = Math.min(w, h) * 0.65f;
            float cx = w / 2f;
            float cy = h / 2f;
            float left = cx - side / 2f;
            float top = cy - side / 2f;
            float right = cx + side / 2f;
            float bottom = cy + side / 2f;

            // Draw scrim around the cutout (4 rectangles)
            canvas.drawRect(0, 0, w, top, scrimPaint);          // top
            canvas.drawRect(0, bottom, w, h, scrimPaint);       // bottom
            canvas.drawRect(0, top, left, bottom, scrimPaint);  // left
            canvas.drawRect(right, top, w, bottom, scrimPaint); // right

            // Corner bracket length
            float arm = 32 * dp;

            // Draw 4 corner brackets
            Path path = new Path();

            // Top-left
            path.moveTo(left, top + arm);
            path.lineTo(left, top);
            path.lineTo(left + arm, top);

            // Top-right
            path.moveTo(right - arm, top);
            path.lineTo(right, top);
            path.lineTo(right, top + arm);

            // Bottom-right
            path.moveTo(right, bottom - arm);
            path.lineTo(right, bottom);
            path.lineTo(right - arm, bottom);

            // Bottom-left
            path.moveTo(left + arm, bottom);
            path.lineTo(left, bottom);
            path.lineTo(left, bottom - arm);

            canvas.drawPath(path, cornerPaint);
        }
    }
}
