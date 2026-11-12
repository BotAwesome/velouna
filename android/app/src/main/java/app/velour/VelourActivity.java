package app.velour;

import android.app.NativeActivity;
import android.os.Build;
import android.view.View;
import android.view.Window;
import android.view.WindowInsets;
import android.view.WindowInsetsController;
import android.view.WindowManager;

/**
 * NativeActivity plus the few window calls that must run on the UI thread.
 * The Rust side (android_main thread) calls these through JNI.
 */
public class VelourActivity extends NativeActivity {

    /** Keep the display on while media plays. */
    public void setKeepScreenOn(final boolean on) {
        runOnUiThread(() -> {
            Window w = getWindow();
            if (on) {
                w.addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON);
            } else {
                w.clearFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON);
            }
        });
    }

    /** setRequestedOrientation on the UI thread (-1 = back to the system). */
    public void setOrientation(final int o) {
        runOnUiThread(() -> setRequestedOrientation(o));
    }

    /** Window brightness 0..1 for this app; negative = follow the system. */
    public void setBrightness(final float v) {
        runOnUiThread(() -> {
            Window w = getWindow();
            WindowManager.LayoutParams lp = w.getAttributes();
            lp.screenBrightness = v < 0 ? WindowManager.LayoutParams.BRIGHTNESS_OVERRIDE_NONE : Math.max(0.01f, Math.min(1f, v));
            w.setAttributes(lp);
        });
    }

    /** The window's brightness, or the system setting mapped to 0..1. */
    public float getBrightness() {
        float own = getWindow().getAttributes().screenBrightness;
        if (own >= 0) return own;
        try {
            int v = android.provider.Settings.System.getInt(getContentResolver(), android.provider.Settings.System.SCREEN_BRIGHTNESS);
            int id = android.content.res.Resources.getSystem().getIdentifier("config_screenBrightnessSettingMaximum", "integer", "android");
            int max = id != 0 ? android.content.res.Resources.getSystem().getInteger(id) : 255;
            if (max <= 0) max = 255;
            return Math.max(0f, Math.min(1f, v / (float) max));
        } catch (Exception e) {
            return -1f;
        }
    }

    /**
     * Width in px of the left/right edge strips the system uses for its back
     * gesture (0 with button navigation): {left, right}.
     */
    public int[] getGestureEdges() {
        if (Build.VERSION.SDK_INT < 29) return new int[] {0, 0};
        WindowInsets wi = getWindow().getDecorView().getRootWindowInsets();
        if (wi == null) return new int[] {0, 0};
        android.graphics.Insets g = wi.getSystemGestureInsets();
        return new int[] {g.left, g.right};
    }

    /**
     * Takes a part of the screen edge back from the system back gesture
     * (Android allows up to 200 dp per edge). Width 0 clears it.
     */
    public void setGestureExclusion(final int x, final int y, final int w, final int h) {
        if (Build.VERSION.SDK_INT < 29) return;
        runOnUiThread(() -> {
            java.util.List<android.graphics.Rect> rects = new java.util.ArrayList<>();
            if (w > 0 && h > 0) rects.add(new android.graphics.Rect(x, y, x + w, y + h));
            getWindow().getDecorView().setSystemGestureExclusionRects(rects);
        });
    }

    // ------------------------------------------------------------ updates

    private static final String ACTION_INSTALL = "app.velour.INSTALL_STATUS";
    /** -100 nothing started, -1 running / waiting for the user, else PackageInstaller.STATUS_*. */
    private volatile int installStatus = -100;
    private volatile String installMessage = "";

    /**
     * Installs a downloaded APK over this app with the system package
     * installer; the user confirms in the system dialog (and allows Velour
     * as a source the first time).
     */
    public void installApk(final String path) {
        installStatus = -1;
        installMessage = "";
        new Thread(() -> {
            try {
                java.io.File f = new java.io.File(path);
                android.content.pm.PackageInstaller pi = getPackageManager().getPackageInstaller();
                android.content.pm.PackageInstaller.SessionParams params =
                        new android.content.pm.PackageInstaller.SessionParams(android.content.pm.PackageInstaller.SessionParams.MODE_FULL_INSTALL);
                params.setAppPackageName(getPackageName());
                int id = pi.createSession(params);
                try (android.content.pm.PackageInstaller.Session s = pi.openSession(id)) {
                    try (java.io.InputStream in = new java.io.FileInputStream(f);
                         java.io.OutputStream out = s.openWrite("velour.apk", 0, f.length())) {
                        byte[] buf = new byte[1 << 16];
                        int n;
                        while ((n = in.read(buf)) > 0) out.write(buf, 0, n);
                        s.fsync(out);
                    }
                    android.content.Intent i = new android.content.Intent(this, VelourActivity.class).setAction(ACTION_INSTALL);
                    // FLAG_MUTABLE (API 31+): the installer fills in the result.
                    int flags = android.app.PendingIntent.FLAG_UPDATE_CURRENT | (Build.VERSION.SDK_INT >= 31 ? 0x02000000 : 0);
                    android.app.PendingIntent p = android.app.PendingIntent.getActivity(this, id, i, flags);
                    s.commit(p.getIntentSender());
                }
            } catch (Exception e) {
                installMessage = String.valueOf(e);
                installStatus = 1;
            }
        }).start();
    }

    @Override
    @SuppressWarnings("deprecation")
    protected void onNewIntent(android.content.Intent intent) {
        super.onNewIntent(intent);
        if (intent == null || !ACTION_INSTALL.equals(intent.getAction())) return;
        int status = intent.getIntExtra(android.content.pm.PackageInstaller.EXTRA_STATUS, 1);
        if (status == android.content.pm.PackageInstaller.STATUS_PENDING_USER_ACTION) {
            android.content.Intent confirm = intent.getParcelableExtra(android.content.Intent.EXTRA_INTENT);
            if (confirm != null) {
                try {
                    startActivity(confirm);
                } catch (Exception e) {
                    installMessage = String.valueOf(e);
                    installStatus = 1;
                }
            }
            return;
        }
        String msg = intent.getStringExtra(android.content.pm.PackageInstaller.EXTRA_STATUS_MESSAGE);
        installMessage = msg == null ? "" : msg;
        installStatus = status;
    }

    /** Velour may install APKs ("install unknown apps" for this app). */
    public boolean canInstallPackages() {
        return getPackageManager().canRequestPackageInstalls();
    }

    /** Opens the system page where the user allows Velour to install apps. */
    public void openInstallPermission() {
        runOnUiThread(() -> {
            try {
                startActivity(new android.content.Intent(android.provider.Settings.ACTION_MANAGE_UNKNOWN_APP_SOURCES,
                        android.net.Uri.parse("package:" + getPackageName())));
            } catch (Exception e) {
                installMessage = String.valueOf(e);
            }
        });
    }

    public int getInstallStatus() {
        return installStatus;
    }

    public String getInstallMessage() {
        return installMessage;
    }

    /** Hide status and navigation bar (swipe brings them back briefly). */
    @SuppressWarnings("deprecation")
    public void setImmersive(final boolean on) {
        runOnUiThread(() -> {
            Window w = getWindow();
            if (Build.VERSION.SDK_INT >= 30) {
                WindowInsetsController c = w.getInsetsController();
                if (c == null) return;
                if (on) {
                    c.setSystemBarsBehavior(WindowInsetsController.BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE);
                    c.hide(WindowInsets.Type.systemBars());
                } else {
                    c.show(WindowInsets.Type.systemBars());
                }
            } else {
                View d = w.getDecorView();
                d.setSystemUiVisibility(on
                        ? View.SYSTEM_UI_FLAG_IMMERSIVE_STICKY | View.SYSTEM_UI_FLAG_FULLSCREEN
                          | View.SYSTEM_UI_FLAG_HIDE_NAVIGATION | View.SYSTEM_UI_FLAG_LAYOUT_STABLE
                          | View.SYSTEM_UI_FLAG_LAYOUT_FULLSCREEN | View.SYSTEM_UI_FLAG_LAYOUT_HIDE_NAVIGATION
                        : 0);
            }
        });
    }
}
