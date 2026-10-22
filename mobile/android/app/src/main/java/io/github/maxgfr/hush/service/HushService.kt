package io.github.maxgfr.hush.service

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.os.IBinder
import androidx.core.app.NotificationCompat
import androidx.core.app.ServiceCompat
import androidx.core.content.ContextCompat
import io.github.maxgfr.hush.MainActivity
import io.github.maxgfr.hush.R

/**
 * Keeps the process (the engine and its radios) alive while a conversation is open or the radios
 * are looking for someone. Its notification never says who or what: only that hush is running.
 */
class HushService : Service() {
    override fun onBind(intent: Intent?): IBinder? = null

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        val manager = getSystemService(NotificationManager::class.java)
        manager.createNotificationChannel(
            NotificationChannel(CHANNEL, getString(R.string.notif_channel), NotificationManager.IMPORTANCE_LOW).apply {
                setShowBadge(false)
                lockscreenVisibility = Notification.VISIBILITY_SECRET
            },
        )
        val open = PendingIntent.getActivity(
            this,
            0,
            Intent(this, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
        val notification = NotificationCompat.Builder(this, CHANNEL)
            .setSmallIcon(R.drawable.ic_notification)
            .setContentTitle(getString(R.string.notif_title))
            .setContentText(getString(R.string.notif_text))
            .setContentIntent(open)
            .setOngoing(true)
            .setVisibility(NotificationCompat.VISIBILITY_SECRET)
            .setCategory(NotificationCompat.CATEGORY_SERVICE)
            .build()
        try {
            ServiceCompat.startForeground(this, ID, notification, ServiceInfo.FOREGROUND_SERVICE_TYPE_CONNECTED_DEVICE)
        } catch (_: RuntimeException) {
            // Not allowed right now (background start, missing prerequisite): hush still works while open.
            stopSelf()
        }
        return START_NOT_STICKY
    }

    companion object {
        private const val CHANNEL = "running"
        private const val ID = 1

        /** Starts or stops the service; refused quietly while the app is in the background. */
        fun set(context: Context, on: Boolean) {
            val intent = Intent(context, HushService::class.java)
            try {
                if (on) ContextCompat.startForegroundService(context, intent) else context.stopService(intent)
            } catch (_: IllegalStateException) {
            } catch (_: SecurityException) {
            }
        }
    }
}
