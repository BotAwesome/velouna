package app.velour;

import android.app.Notification;
import android.app.NotificationChannel;
import android.app.NotificationManager;
import android.app.PendingIntent;
import android.app.Service;
import android.content.Context;
import android.content.Intent;
import android.content.pm.ServiceInfo;
import android.graphics.Bitmap;
import android.graphics.drawable.Icon;
import android.media.session.MediaSession;
import android.os.Build;
import android.os.IBinder;

/**
 * Foreground service behind the player notification: keeps playback
 * running while Velour is in the background and shows the media controls
 * (notification shade, lock screen). The media session itself lives in
 * {@link VelourActivity}; this only owns the notification.
 */
public class PlaybackService extends Service {

    static final String CHANNEL = "playback";
    static final int ID = 1;
    static final String ACTION_PREV = "app.velour.PREV";
    static final String ACTION_TOGGLE = "app.velour.TOGGLE";
    static final String ACTION_NEXT = "app.velour.NEXT";
    static final String ACTION_STOP = "app.velour.STOP";

    /** Labels from the Rust side (UI language): prev, play, pause, next, channel. */
    static volatile String[] labels = {"Previous", "Play", "Pause", "Next", "Playback"};

    private static volatile Notification current;
    private static volatile boolean foreground;

    /** Shows or updates the notification; playing keeps the service in the foreground. */
    static void show(Context c, MediaSession.Token token, String title, String text, boolean playing, Bitmap art) {
        ensureChannel(c);
        Notification n = build(c, token, title, text, playing, art);
        current = n;
        NotificationManager nm = c.getSystemService(NotificationManager.class);
        if (playing && !foreground) {
            try {
                c.startForegroundService(new Intent(c, PlaybackService.class));
                return;
            } catch (Exception e) {
                // Not allowed from the background right now: plain notification.
            }
        }
        if (nm != null) nm.notify(ID, n);
    }

    /** Removes the notification and ends the service. */
    static void hide(Context c) {
        current = null;
        foreground = false;
        c.stopService(new Intent(c, PlaybackService.class));
        NotificationManager nm = c.getSystemService(NotificationManager.class);
        if (nm != null) nm.cancel(ID);
    }

    /** Paused: the notification stays but can be swiped away. */
    static void detach(Context c) {
        if (!foreground) return;
        foreground = false;
        Intent i = new Intent(c, PlaybackService.class).setAction("detach");
        try {
            c.startService(i);
        } catch (Exception e) {
            // The service is running (foreground until now), so this works;
            // nothing to do otherwise.
        }
    }

    private static void ensureChannel(Context c) {
        NotificationManager nm = c.getSystemService(NotificationManager.class);
        if (nm == null) return;
        NotificationChannel ch = new NotificationChannel(CHANNEL, labels[4], NotificationManager.IMPORTANCE_LOW);
        ch.setShowBadge(false);
        ch.setLockscreenVisibility(Notification.VISIBILITY_PUBLIC);
        nm.createNotificationChannel(ch);
    }

    private static PendingIntent action(Context c, String action, int code) {
        Intent i = new Intent(c, PlaybackService.class).setAction(action);
        return PendingIntent.getService(c, code, i, PendingIntent.FLAG_IMMUTABLE | PendingIntent.FLAG_UPDATE_CURRENT);
    }

    private static Notification build(Context c, MediaSession.Token token, String title, String text, boolean playing, Bitmap art) {
        String[] l = labels;
        Intent open = new Intent(c, VelourActivity.class).addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP);
        PendingIntent content = PendingIntent.getActivity(c, 0, open, PendingIntent.FLAG_IMMUTABLE | PendingIntent.FLAG_UPDATE_CURRENT);
        Notification.Builder b = new Notification.Builder(c, CHANNEL)
                .setSmallIcon(R.drawable.ic_notification)
                .setContentTitle(title)
                .setContentText(text)
                .setContentIntent(content)
                .setDeleteIntent(action(c, ACTION_STOP, 4))
                .setVisibility(Notification.VISIBILITY_PUBLIC)
                .setCategory(Notification.CATEGORY_TRANSPORT)
                .setOnlyAlertOnce(true)
                .setShowWhen(false)
                .setOngoing(playing)
                .addAction(new Notification.Action.Builder(Icon.createWithResource(c, android.R.drawable.ic_media_previous), l[0], action(c, ACTION_PREV, 1)).build())
                .addAction(new Notification.Action.Builder(Icon.createWithResource(c, playing ? android.R.drawable.ic_media_pause : android.R.drawable.ic_media_play), playing ? l[2] : l[1], action(c, ACTION_TOGGLE, 2)).build())
                .addAction(new Notification.Action.Builder(Icon.createWithResource(c, android.R.drawable.ic_media_next), l[3], action(c, ACTION_NEXT, 3)).build())
                .setStyle(new Notification.MediaStyle().setMediaSession(token).setShowActionsInCompactView(0, 1, 2));
        if (art != null) b.setLargeIcon(art);
        return b.build();
    }

    @Override
    public int onStartCommand(Intent intent, int flags, int startId) {
        String a = intent == null ? null : intent.getAction();
        if (ACTION_PREV.equals(a)) {
            VelourActivity.mediaAction(VelourActivity.MEDIA_PREV, 0);
        } else if (ACTION_TOGGLE.equals(a)) {
            VelourActivity.mediaAction(VelourActivity.MEDIA_TOGGLE, 0);
        } else if (ACTION_NEXT.equals(a)) {
            VelourActivity.mediaAction(VelourActivity.MEDIA_NEXT, 0);
        } else if (ACTION_STOP.equals(a)) {
            // Swiped away while paused.
            VelourActivity.mediaAction(VelourActivity.MEDIA_PAUSE, 0);
            current = null;
            foreground = false;
            stopSelf();
            return START_NOT_STICKY;
        } else if ("detach".equals(a)) {
            stopForeground(STOP_FOREGROUND_DETACH);
            return START_NOT_STICKY;
        }
        Notification n = current;
        if (a == null && n == null) {
            // Hidden again right after it was shown: Android still wants
            // startForeground after startForegroundService.
            ensureChannel(this);
            n = new Notification.Builder(this, CHANNEL).setSmallIcon(R.drawable.ic_notification).build();
            try {
                startForeground(ID, n);
            } catch (Exception ignored) {
            }
            stopForeground(STOP_FOREGROUND_REMOVE);
            stopSelf();
            return START_NOT_STICKY;
        }
        if (a == null) {
            try {
                if (Build.VERSION.SDK_INT >= 29) {
                    startForeground(ID, n, ServiceInfo.FOREGROUND_SERVICE_TYPE_MEDIA_PLAYBACK);
                } else {
                    startForeground(ID, n);
                }
                foreground = true;
            } catch (Exception e) {
                NotificationManager nm = getSystemService(NotificationManager.class);
                if (nm != null) nm.notify(ID, n);
            }
        }
        return START_NOT_STICKY;
    }

    /** Swiped out of the recent apps: stop playing, nothing left to control. */
    @Override
    public void onTaskRemoved(Intent rootIntent) {
        VelourActivity.mediaAction(VelourActivity.MEDIA_PAUSE, 0);
        hide(this);
        super.onTaskRemoved(rootIntent);
    }

    @Override
    public void onDestroy() {
        foreground = false;
        super.onDestroy();
    }

    @Override
    public IBinder onBind(Intent intent) {
        return null;
    }
}
