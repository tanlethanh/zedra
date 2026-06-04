package dev.zedra.app;

import android.app.Notification;
import android.app.NotificationManager;
import android.app.PendingIntent;
import android.content.Intent;
import android.net.Uri;
import android.util.Log;

import androidx.core.app.NotificationCompat;

import com.google.firebase.messaging.FirebaseMessagingService;
import com.google.firebase.messaging.RemoteMessage;

import java.util.Map;

/**
 * Receives Delta push messages from Firebase Cloud Messaging and posts them as
 * system notifications. Foreground in-app banners are handled separately by
 * MainActivity.showNativeNotification via the platform bridge.
 */
public class ZedraMessagingService extends FirebaseMessagingService {
    private static final String TAG = "ZedraMessaging";

    // Must match MainActivity.DELTA_NOTIFICATION_CHANNEL_ID.
    static final String CHANNEL_ID = "zedra_delta";

    @Override
    public void onNewToken(String token) {
        // The app fetches the current token on demand via requestDeltaPushToken,
        // so a rotated token is picked up on the next registration. Log only.
        Log.d(TAG, "FCM token refreshed");
    }

    @Override
    public void onMessageReceived(RemoteMessage message) {
        String title = null;
        String body = null;
        if (message.getNotification() != null) {
            title = message.getNotification().getTitle();
            body = message.getNotification().getBody();
        }

        Map<String, String> data = message.getData();
        if (title == null) {
            title = data.get("title");
        }
        if (body == null) {
            body = data.get("body");
        }
        if (title == null && body == null) {
            return;
        }

        Intent intent = new Intent(this, MainActivity.class);
        intent.setFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP | Intent.FLAG_ACTIVITY_CLEAR_TOP);
        String deeplink = data.get("deeplink");
        if (deeplink != null && !deeplink.isEmpty()) {
            intent.setAction(Intent.ACTION_VIEW);
            intent.setData(Uri.parse(deeplink));
        }

        PendingIntent pendingIntent = PendingIntent.getActivity(
            this,
            0,
            intent,
            PendingIntent.FLAG_UPDATE_CURRENT | PendingIntent.FLAG_IMMUTABLE);

        NotificationCompat.Builder builder = new NotificationCompat.Builder(this, CHANNEL_ID)
            .setSmallIcon(R.mipmap.ic_launcher)
            .setContentTitle(title != null ? title : "Zedra")
            .setContentText(body)
            .setStyle(new NotificationCompat.BigTextStyle().bigText(body))
            .setAutoCancel(true)
            .setPriority(NotificationCompat.PRIORITY_HIGH)
            .setContentIntent(pendingIntent);

        NotificationManager manager =
            (NotificationManager) getSystemService(NOTIFICATION_SERVICE);
        if (manager != null) {
            manager.notify((int) (System.currentTimeMillis() % Integer.MAX_VALUE), builder.build());
        }
    }
}
