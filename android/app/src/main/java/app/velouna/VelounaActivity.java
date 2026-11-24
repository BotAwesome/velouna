package app.velouna;

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
public class VelounaActivity extends NativeActivity {

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
     * Space in px taken by system bars, display cutout and keyboard:
     * {left, top, right, bottom}. From the window metrics (API 30+), which
     * follow a rotation right away; the decor view's root insets, read from
     * the native thread, can stay at the old orientation.
     */
    @SuppressWarnings("deprecation")
    public int[] getSafeInsets() {
        WindowInsets root = getWindow().getDecorView().getRootWindowInsets();
        if (Build.VERSION.SDK_INT >= 30) {
            WindowInsets wi = getWindowManager().getCurrentWindowMetrics().getWindowInsets();
            android.graphics.Insets s = wi.getInsets(WindowInsets.Type.systemBars() | WindowInsets.Type.displayCutout());
            int bottom = s.bottom;
            if (root != null) bottom = Math.max(bottom, root.getInsets(WindowInsets.Type.ime()).bottom);
            return new int[] {s.left, s.top, s.right, bottom};
        }
        if (root == null) return new int[] {0, 0, 0, 0};
        return new int[] {root.getSystemWindowInsetLeft(), root.getSystemWindowInsetTop(), root.getSystemWindowInsetRight(), root.getSystemWindowInsetBottom()};
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

    private static final String ACTION_INSTALL = "app.velouna.INSTALL_STATUS";
    /** -100 nothing started, -1 running / waiting for the user, else PackageInstaller.STATUS_*. */
    private volatile int installStatus = -100;
    private volatile String installMessage = "";

    /**
     * Installs a downloaded APK over this app with the system package
     * installer; the user confirms in the system dialog (and allows Velouna
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
                         java.io.OutputStream out = s.openWrite("velouna.apk", 0, f.length())) {
                        byte[] buf = new byte[1 << 16];
                        int n;
                        while ((n = in.read(buf)) > 0) out.write(buf, 0, n);
                        s.fsync(out);
                    }
                    android.content.Intent i = new android.content.Intent(this, VelounaActivity.class).setAction(ACTION_INSTALL);
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

    /** Velouna may install APKs ("install unknown apps" for this app). */
    public boolean canInstallPackages() {
        return getPackageManager().canRequestPackageInstalls();
    }

    /** Opens the system page where the user allows Velouna to install apps. */
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

    // ------------------------------------------------------------ storage

    private static final int REQ_STORAGE = 7;

    private static String[] storagePermissions() {
        if (Build.VERSION.SDK_INT >= 33) {
            return new String[] {"android.permission.READ_MEDIA_VIDEO", "android.permission.READ_MEDIA_AUDIO"};
        }
        return new String[] {"android.permission.READ_EXTERNAL_STORAGE"};
    }

    /**
     * What Velouna may read: bit 1 videos, bit 2 music, bit 4 all files
     * (subtitles and playlists next to the media as well).
     */
    public int storageAccess() {
        int bits = 0;
        if (Build.VERSION.SDK_INT >= 33) {
            if (checkSelfPermission("android.permission.READ_MEDIA_VIDEO") == android.content.pm.PackageManager.PERMISSION_GRANTED) bits |= 1;
            if (checkSelfPermission("android.permission.READ_MEDIA_AUDIO") == android.content.pm.PackageManager.PERMISSION_GRANTED) bits |= 2;
        } else if (checkSelfPermission("android.permission.READ_EXTERNAL_STORAGE") == android.content.pm.PackageManager.PERMISSION_GRANTED) {
            bits |= 3;
        }
        if (Build.VERSION.SDK_INT >= 30) {
            if (android.os.Environment.isExternalStorageManager()) bits |= 7;
        } else if ((bits & 3) == 3) {
            // Before Android 11 the storage permission covers every file.
            bits |= 4;
        }
        return bits;
    }

    /** Velouna asked for the storage permission before (shown once by itself). */
    public boolean storageAsked() {
        return getPreferences(MODE_PRIVATE).getBoolean("storage_asked", false);
    }

    /**
     * Asks for the media permissions; once Android no longer shows the
     * dialog (denied twice), opens Velouna's page in the system settings.
     */
    public void requestStorage() {
        runOnUiThread(() -> {
            java.util.List<String> missing = new java.util.ArrayList<>();
            boolean rationale = false;
            for (String p : storagePermissions()) {
                if (checkSelfPermission(p) != android.content.pm.PackageManager.PERMISSION_GRANTED) {
                    missing.add(p);
                    rationale |= shouldShowRequestPermissionRationale(p);
                }
            }
            if (missing.isEmpty()) return;
            android.content.SharedPreferences prefs = getPreferences(MODE_PRIVATE);
            if (prefs.getBoolean("storage_asked", false) && !rationale) {
                openAppSettings();
                return;
            }
            prefs.edit().putBoolean("storage_asked", true).apply();
            requestPermissions(missing.toArray(new String[0]), REQ_STORAGE);
        });
    }

    /** "All files access" (Android 11+); before that the normal storage dialog. */
    public void openAllFilesAccess() {
        if (Build.VERSION.SDK_INT < 30) {
            requestStorage();
            return;
        }
        runOnUiThread(() -> {
            try {
                startActivity(new android.content.Intent(android.provider.Settings.ACTION_MANAGE_APP_ALL_FILES_ACCESS_PERMISSION,
                        android.net.Uri.parse("package:" + getPackageName())));
            } catch (Exception e) {
                try {
                    startActivity(new android.content.Intent(android.provider.Settings.ACTION_MANAGE_ALL_FILES_ACCESS_PERMISSION));
                } catch (Exception ignored) {
                }
            }
        });
    }

    private void openAppSettings() {
        try {
            startActivity(new android.content.Intent(android.provider.Settings.ACTION_APPLICATION_DETAILS_SETTINGS,
                    android.net.Uri.parse("package:" + getPackageName())));
        } catch (Exception ignored) {
        }
    }

    // ------------------------------------------------------------ picker

    private static final int REQ_PICK = 41;
    /** Media files (play / queue / playlist). */
    static final int PICK_MEDIA = 0;
    /** Subtitle files (copied into the cache when the file itself can't be read). */
    static final int PICK_SUBTITLES = 1;
    /** A folder. */
    static final int PICK_FOLDER = 2;

    private volatile String[] pickResult;
    private volatile int pickMode;

    /**
     * Opens the system picker. The answer arrives in {@link #takePick()}:
     * file paths (or fdclose://N for files without one), "!name" for an
     * item Velouna can't open, nothing when cancelled.
     */
    public void pick(final int mode, final boolean multiple) {
        pickResult = null;
        pickMode = mode;
        runOnUiThread(() -> {
            android.content.Intent i;
            if (mode == PICK_FOLDER) {
                i = new android.content.Intent(android.content.Intent.ACTION_OPEN_DOCUMENT_TREE);
            } else {
                i = new android.content.Intent(android.content.Intent.ACTION_OPEN_DOCUMENT);
                i.addCategory(android.content.Intent.CATEGORY_OPENABLE);
                i.setType("*/*");
                if (mode == PICK_MEDIA) {
                    i.putExtra(android.content.Intent.EXTRA_MIME_TYPES, new String[] {
                            "video/*", "audio/*", "application/x-mpegurl", "audio/x-mpegurl", "application/vnd.apple.mpegurl", "audio/x-scpls"});
                }
                i.putExtra(android.content.Intent.EXTRA_ALLOW_MULTIPLE, multiple);
            }
            try {
                startActivityForResult(i, REQ_PICK);
            } catch (Exception e) {
                pickResult = new String[0];
            }
        });
    }

    /** The picker's answer once, or null while it is still open. */
    public String[] takePick() {
        String[] r = pickResult;
        if (r != null) pickResult = null;
        return r;
    }

    @Override
    protected void onActivityResult(int requestCode, int resultCode, android.content.Intent data) {
        super.onActivityResult(requestCode, resultCode, data);
        if (requestCode != REQ_PICK) return;
        if (resultCode != RESULT_OK || data == null) {
            pickResult = new String[0];
            return;
        }
        final java.util.List<android.net.Uri> uris = new java.util.ArrayList<>();
        android.content.ClipData clip = data.getClipData();
        if (clip != null) {
            for (int k = 0; k < clip.getItemCount(); k++) uris.add(clip.getItemAt(k).getUri());
        } else if (data.getData() != null) {
            uris.add(data.getData());
        }
        final int mode = pickMode;
        // Resolving (and copying subtitles) may take a moment: off the UI thread.
        new Thread(() -> {
            java.util.List<String> out = new java.util.ArrayList<>();
            for (android.net.Uri u : uris) {
                String s = resolvePicked(u, mode);
                if (s != null) out.add(s);
            }
            pickResult = out.toArray(new String[0]);
        }).start();
    }

    private String resolvePicked(android.net.Uri uri, int mode) {
        String name = displayName(uri);
        if (mode == PICK_FOLDER) {
            String p = pathOf(uri, true);
            return p != null ? p : "!" + name;
        }
        String p = pathOf(uri, false);
        if (p != null && new java.io.File(p).canRead()) return p;
        if (mode == PICK_SUBTITLES) {
            // Without "all files access" Android hides .srt files from the
            // file system: a copy in the cache keeps its name for matching.
            try {
                java.io.File dir = new java.io.File(getCacheDir(), "subtitles");
                dir.mkdirs();
                java.io.File f = new java.io.File(dir, name.replace('/', '_'));
                try (java.io.InputStream in = getContentResolver().openInputStream(uri);
                     java.io.OutputStream o = new java.io.FileOutputStream(f)) {
                    if (in == null) return "!" + name;
                    byte[] buf = new byte[1 << 15];
                    int n;
                    while ((n = in.read(buf)) > 0) o.write(buf, 0, n);
                }
                return f.getPath();
            } catch (Exception e) {
                return "!" + name;
            }
        }
        // A file without a path (cloud storage, …): mpv reads the descriptor.
        try {
            android.os.ParcelFileDescriptor pfd = getContentResolver().openFileDescriptor(uri, "r");
            if (pfd == null) return "!" + name;
            return "fdclose://" + pfd.detachFd();
        } catch (Exception e) {
            return "!" + name;
        }
    }

    private String displayName(android.net.Uri uri) {
        try (android.database.Cursor c = getContentResolver().query(uri, new String[] {android.provider.OpenableColumns.DISPLAY_NAME}, null, null, null)) {
            if (c != null && c.moveToFirst() && c.getString(0) != null) return c.getString(0);
        } catch (Exception ignored) {
        }
        String s = uri.getLastPathSegment();
        if (s == null) return "?";
        int colon = s.lastIndexOf(':');
        int slash = s.lastIndexOf('/');
        return s.substring(Math.max(colon, slash) + 1);
    }

    /** File system path behind a picker URI, where there is one. */
    private String pathOf(android.net.Uri uri, boolean tree) {
        try {
            if ("file".equals(uri.getScheme())) return uri.getPath();
            String auth = uri.getAuthority();
            String id;
            if (tree) {
                id = android.provider.DocumentsContract.getTreeDocumentId(uri);
            } else if (android.provider.DocumentsContract.isDocumentUri(this, uri)) {
                id = android.provider.DocumentsContract.getDocumentId(uri);
            } else {
                return queryData(uri);
            }
            if ("com.android.externalstorage.documents".equals(auth)) {
                int c = id.indexOf(':');
                String vol = c < 0 ? id : id.substring(0, c);
                String rel = c < 0 ? "" : id.substring(c + 1);
                String base = "primary".equalsIgnoreCase(vol)
                        ? android.os.Environment.getExternalStorageDirectory().getPath()
                        : "/storage/" + vol;
                return rel.isEmpty() ? base : base + "/" + rel;
            }
            if ("com.android.providers.downloads.documents".equals(auth)) {
                if (id.startsWith("raw:")) return id.substring(4);
                if (id.equals("downloads")) {
                    return android.os.Environment.getExternalStoragePublicDirectory(android.os.Environment.DIRECTORY_DOWNLOADS).getPath();
                }
                if (id.startsWith("msf:")) {
                    return queryData(android.content.ContentUris.withAppendedId(android.provider.MediaStore.Files.getContentUri("external"), Long.parseLong(id.substring(4))));
                }
                return null;
            }
            if ("com.android.providers.media.documents".equals(auth)) {
                String[] p = id.split(":");
                android.net.Uri base;
                switch (p[0]) {
                    case "video": base = android.provider.MediaStore.Video.Media.EXTERNAL_CONTENT_URI; break;
                    case "audio": base = android.provider.MediaStore.Audio.Media.EXTERNAL_CONTENT_URI; break;
                    default: base = android.provider.MediaStore.Files.getContentUri("external");
                }
                return queryData(android.content.ContentUris.withAppendedId(base, Long.parseLong(p[1])));
            }
        } catch (Exception ignored) {
        }
        return null;
    }

    @SuppressWarnings("deprecation")
    private String queryData(android.net.Uri uri) {
        try (android.database.Cursor c = getContentResolver().query(uri, new String[] {android.provider.MediaStore.MediaColumns.DATA}, null, null, null)) {
            if (c != null && c.moveToFirst()) return c.getString(0);
        } catch (Exception ignored) {
        }
        return null;
    }

    // ------------------------------------------------------------ media session

    static final int MEDIA_TOGGLE = 0;
    static final int MEDIA_PLAY = 1;
    static final int MEDIA_PAUSE = 2;
    static final int MEDIA_NEXT = 3;
    static final int MEDIA_PREV = 4;
    static final int MEDIA_SEEK = 5;

    /** Implemented in Rust (registered at start): runs the command on mpv. */
    private static native void nativeMediaAction(int action, long arg);

    /** Notification, lock screen, headset buttons, audio focus → mpv. */
    static void mediaAction(int action, long arg) {
        try {
            nativeMediaAction(action, arg);
        } catch (Throwable t) {
            android.util.Log.w("Velouna", "media action " + action + ": " + t);
        }
    }

    private android.media.session.MediaSession session;
    private android.media.AudioFocusRequest focusRequest;
    private boolean hasFocus;
    private boolean resumeOnGain;
    private android.content.BroadcastReceiver noisy;
    private String artPath;
    private android.graphics.Bitmap art;

    /** Labels in the UI language: previous, play, pause, next, channel name. */
    public void setMediaLabels(String[] labels) {
        if (labels != null && labels.length == 5) PlaybackService.labels = labels;
    }

    /**
     * Shows what plays in the notification and on the lock screen.
     * artPath: an image file, or null/empty for none.
     */
    public void updateMedia(final String title, final String text, final boolean playing, final long posMs, final long durMs,
                            final float speed, final String artFile) {
        runOnUiThread(() -> {
            if (session == null) {
                session = new android.media.session.MediaSession(this, "Velouna");
                session.setCallback(new android.media.session.MediaSession.Callback() {
                    @Override public void onPlay() { mediaAction(MEDIA_PLAY, 0); }
                    @Override public void onPause() { mediaAction(MEDIA_PAUSE, 0); }
                    @Override public void onStop() { mediaAction(MEDIA_PAUSE, 0); }
                    @Override public void onSkipToNext() { mediaAction(MEDIA_NEXT, 0); }
                    @Override public void onSkipToPrevious() { mediaAction(MEDIA_PREV, 0); }
                    @Override public void onSeekTo(long pos) { mediaAction(MEDIA_SEEK, pos); }
                });
                android.content.Intent open = new android.content.Intent(this, VelounaActivity.class).addFlags(android.content.Intent.FLAG_ACTIVITY_SINGLE_TOP);
                session.setSessionActivity(android.app.PendingIntent.getActivity(this, 0, open,
                        android.app.PendingIntent.FLAG_IMMUTABLE | android.app.PendingIntent.FLAG_UPDATE_CURRENT));
            }
            String file = artFile == null || artFile.isEmpty() ? null : artFile;
            if (file == null ? artPath != null : !file.equals(artPath)) {
                artPath = file;
                art = file == null ? null : android.graphics.BitmapFactory.decodeFile(file);
            }
            android.media.MediaMetadata.Builder m = new android.media.MediaMetadata.Builder()
                    .putString(android.media.MediaMetadata.METADATA_KEY_TITLE, title)
                    .putString(android.media.MediaMetadata.METADATA_KEY_ARTIST, text)
                    .putLong(android.media.MediaMetadata.METADATA_KEY_DURATION, durMs > 0 ? durMs : -1);
            if (art != null) m.putBitmap(android.media.MediaMetadata.METADATA_KEY_ART, art);
            session.setMetadata(m.build());
            long actions = android.media.session.PlaybackState.ACTION_PLAY | android.media.session.PlaybackState.ACTION_PAUSE
                    | android.media.session.PlaybackState.ACTION_PLAY_PAUSE | android.media.session.PlaybackState.ACTION_STOP
                    | android.media.session.PlaybackState.ACTION_SKIP_TO_NEXT | android.media.session.PlaybackState.ACTION_SKIP_TO_PREVIOUS;
            if (durMs > 0) actions |= android.media.session.PlaybackState.ACTION_SEEK_TO;
            session.setPlaybackState(new android.media.session.PlaybackState.Builder()
                    .setActions(actions)
                    .setState(playing ? android.media.session.PlaybackState.STATE_PLAYING : android.media.session.PlaybackState.STATE_PAUSED,
                            Math.max(0, posMs), playing ? speed : 0f)
                    .build());
            session.setActive(true);
            if (playing) {
                requestFocus();
                listenNoisy(true);
            } else {
                listenNoisy(false);
            }
            PlaybackService.show(this, session.getSessionToken(), title, text, playing, art);
            if (!playing) PlaybackService.detach(this);
        });
    }

    /** Nothing loaded any more: notification away, audio focus back. */
    public void stopMedia() {
        runOnUiThread(() -> {
            if (session != null) session.setActive(false);
            listenNoisy(false);
            abandonFocus();
            PlaybackService.hide(this);
        });
    }

    private void requestFocus() {
        if (hasFocus) return;
        android.media.AudioManager am = getSystemService(android.media.AudioManager.class);
        if (am == null) return;
        if (focusRequest == null) {
            android.media.AudioAttributes aa = new android.media.AudioAttributes.Builder()
                    .setUsage(android.media.AudioAttributes.USAGE_MEDIA)
                    .setContentType(android.media.AudioAttributes.CONTENT_TYPE_MOVIE)
                    .build();
            focusRequest = new android.media.AudioFocusRequest.Builder(android.media.AudioManager.AUDIOFOCUS_GAIN)
                    .setAudioAttributes(aa)
                    .setOnAudioFocusChangeListener(this::onFocusChange)
                    .build();
        }
        hasFocus = am.requestAudioFocus(focusRequest) == android.media.AudioManager.AUDIOFOCUS_REQUEST_GRANTED;
    }

    private void abandonFocus() {
        android.media.AudioManager am = getSystemService(android.media.AudioManager.class);
        if (am != null && focusRequest != null) am.abandonAudioFocusRequest(focusRequest);
        hasFocus = false;
        resumeOnGain = false;
    }

    /** Another app plays (or a call comes in): pause; continue after a short interruption. */
    private void onFocusChange(int change) {
        switch (change) {
            case android.media.AudioManager.AUDIOFOCUS_LOSS:
                hasFocus = false;
                resumeOnGain = false;
                mediaAction(MEDIA_PAUSE, 0);
                break;
            case android.media.AudioManager.AUDIOFOCUS_LOSS_TRANSIENT:
                android.media.session.PlaybackState st = session == null ? null : session.getController().getPlaybackState();
                resumeOnGain = st != null && st.getState() == android.media.session.PlaybackState.STATE_PLAYING;
                mediaAction(MEDIA_PAUSE, 0);
                break;
            case android.media.AudioManager.AUDIOFOCUS_GAIN:
                hasFocus = true;
                if (resumeOnGain) {
                    resumeOnGain = false;
                    mediaAction(MEDIA_PLAY, 0);
                }
                break;
            default:
                // Ducking: Android lowers the volume itself.
        }
    }

    /** Headphones unplugged / Bluetooth gone: pause instead of playing aloud. */
    private void listenNoisy(boolean on) {
        if (on && noisy == null) {
            noisy = new android.content.BroadcastReceiver() {
                @Override
                public void onReceive(android.content.Context c, android.content.Intent i) {
                    mediaAction(MEDIA_PAUSE, 0);
                }
            };
            android.content.IntentFilter f = new android.content.IntentFilter(android.media.AudioManager.ACTION_AUDIO_BECOMING_NOISY);
            if (Build.VERSION.SDK_INT >= 33) {
                registerReceiver(noisy, f, RECEIVER_NOT_EXPORTED);
            } else {
                registerReceiver(noisy, f);
            }
        } else if (!on && noisy != null) {
            try {
                unregisterReceiver(noisy);
            } catch (Exception ignored) {
            }
            noisy = null;
        }
    }

    @Override
    protected void onDestroy() {
        listenNoisy(false);
        abandonFocus();
        PlaybackService.hide(this);
        if (session != null) {
            session.release();
            session = null;
        }
        super.onDestroy();
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

    // ---- media info without ffmpeg ------------------------------------

    private android.media.MediaMetadataRetriever retriever(String path) throws Exception {
        android.media.MediaMetadataRetriever r = new android.media.MediaMetadataRetriever();
        if (path.startsWith("content://")) r.setDataSource(this, android.net.Uri.parse(path));
        else r.setDataSource(path);
        return r;
    }

    /** Tags, duration and size of a file as JSON, or null. */
    public String probeMedia(String path) {
        android.media.MediaMetadataRetriever r = null;
        try {
            r = retriever(path);
            org.json.JSONObject o = new org.json.JSONObject();
            int[][] keys = {
                {android.media.MediaMetadataRetriever.METADATA_KEY_TITLE, 0},
                {android.media.MediaMetadataRetriever.METADATA_KEY_ARTIST, 1},
                {android.media.MediaMetadataRetriever.METADATA_KEY_ALBUMARTIST, 2},
                {android.media.MediaMetadataRetriever.METADATA_KEY_ALBUM, 3},
                {android.media.MediaMetadataRetriever.METADATA_KEY_GENRE, 4},
                {android.media.MediaMetadataRetriever.METADATA_KEY_CD_TRACK_NUMBER, 5},
                {android.media.MediaMetadataRetriever.METADATA_KEY_DURATION, 6},
                {android.media.MediaMetadataRetriever.METADATA_KEY_VIDEO_WIDTH, 7},
                {android.media.MediaMetadataRetriever.METADATA_KEY_VIDEO_HEIGHT, 8},
                {android.media.MediaMetadataRetriever.METADATA_KEY_HAS_VIDEO, 9},
                {android.media.MediaMetadataRetriever.METADATA_KEY_VIDEO_ROTATION, 10},
            };
            String[] names = {"title", "artist", "album_artist", "album", "genre", "track", "duration_ms", "width", "height", "has_video", "rotation"};
            for (int[] k : keys) {
                String v = r.extractMetadata(k[0]);
                if (v != null) o.put(names[k[1]], v);
            }
            o.put("has_cover", r.getEmbeddedPicture() != null);
            return o.toString();
        } catch (Throwable e) {
            return null;
        } finally {
            if (r != null) try { r.release(); } catch (Throwable ignored) {}
        }
    }

    /** One video frame near `us`, at most w×h, as JPEG bytes, or null. */
    public byte[] videoFrame(String path, long us, int w, int h) {
        android.media.MediaMetadataRetriever r = null;
        try {
            r = retriever(path);
            int opt = android.media.MediaMetadataRetriever.OPTION_CLOSEST_SYNC;
            android.graphics.Bitmap b = Build.VERSION.SDK_INT >= 27
                    ? r.getScaledFrameAtTime(us, opt, w, h)
                    : r.getFrameAtTime(us, opt);
            if (b == null) return null;
            java.io.ByteArrayOutputStream out = new java.io.ByteArrayOutputStream();
            b.compress(android.graphics.Bitmap.CompressFormat.JPEG, 85, out);
            b.recycle();
            return out.toByteArray();
        } catch (Throwable e) {
            return null;
        } finally {
            if (r != null) try { r.release(); } catch (Throwable ignored) {}
        }
    }
}
