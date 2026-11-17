# Changelog

All notable changes to Velour. The newest version is at the top; each
section becomes the text of its GitHub release and the "What's new" dialog.

## [0.1.4] - 2026-11-17

### Linux
- Velour now runs on Linux: Pop!_OS, Ubuntu, Debian, Linux Mint and other distributions with
  glibc 2.35 or newer, on Wayland and X11. The .deb installs with apt, the .tar.gz installs for
  your user without root
- Everything is included: mpv, FFmpeg, ffprobe and yt-dlp
- File dialogs of your desktop, the Plex sign-in in the keyring, "Show in file manager" and
  "Open with" from the file manager
- Updates work like on Windows: Velour downloads the new package and asks for your password to
  install it

### Windows and Linux
- Only one Velour window: a file opened while Velour is running goes to the running window,
  and several files opened at once arrive as one list. "Settings → Interface → Window" turns
  this off or adds the files to the queue instead
- The screen stays on while a video plays, and the computer does not go to sleep during music

### Plex
- Your Plex server lists Velour with the right system (it said "Windows" on Android, too)

## [0.1.3] - 2026-11-13

### Android
- Velour asks for access to your videos and music, so the library finds them; if you said no,
  a card in Videos and Music and "Settings → Library" let you allow it later
- "Open file", "Open folder", adding to the queue, subtitles, library folders and playlists use
  the Android file picker
- A player in the notification shade and on the lock screen with previous, play/pause and next;
  music and radio keep playing with the screen off, and pause when headphones are unplugged
- Options that only make sense on a computer are hidden (keyboard shortcuts, FFmpeg, yt-dlp,
  recording and converting, "Show in Explorer")

### Library
- When a played file is gone, Velour asks once whether to remove it from the history and
  lists or keep it, and remembers the answer ("Settings → Library → Missing media"). After
  that it only tells you briefly. A missing USB stick, SD card or network drive removes nothing
- TV channels and radio stations show as live in "Recently played" instead of with a progress

### Plex
- "Continue watching" on the start page shows what Plex lists under "On Deck", including the
  next episode of a show you're watching, most recent first

## [0.1.2] - 2026-11-13

### TV and radio
- TV channels and radio stations have their own pages: in the sidebar on Windows, under "More"
  on Android
- Public TV channels (iptv-org) are sorted by language, with flags, channel logos, search and
  favorites
- What's on now shows next to each channel; "Program guide" in a channel's menu lists today and
  tomorrow (data from epgshare01)
- Radio stations from radio-browser.info replace the single SomaFM stream: by language and
  popularity, with search, favorites and station logos
- The player and the mini player show the song a station is playing

### Playback
- Opening a video with Velour (double-click, "Open with") showed no picture when Velour was not
  running yet; the picture is there now
- Music and radio no longer switch to the full-screen player; they keep playing in the mini
  player while you browse

### Android
- Lists scroll with a swipe, and the view button switches between list and tiles
- The layout follows a rotation right away
- "What's new" shows long changelog lines in one piece

### Website
- Velour has a website: https://botawesome.github.io/velour/

## [0.1.1] - 2026-11-12

### Android
- The app is now signed with its own release key. If you installed 0.1.0, uninstall it once and
  install this APK; later updates install from within the app again
- Updates ask for the "install unknown apps" permission inside Velour and continue right where
  they were after Android restarts the app
- The queue scrolls with a swipe; reorder items with the grip on the right, tap to play

### Playback
- IPTV lists: channel names no longer show parts of the user agent (e.g. "like Gecko) Chrome/…")

## [0.1.0] - 2026-11-12

First public release for Windows and Android.

### Playback
- Plays nearly every video and audio format via mpv, with hardware decoding
- Thick, easy-to-hit seek bar with preview thumbnails, chapter marks and optional waveform
- Seek bar snaps to chapter starts (and stops snapping to a chapter you keep skipping past)
- Resume where you left off, playback speed, A-B repeat, sleep timer, picture-in-picture
- 10-band equalizer with presets, loudness normalization, night mode, video color presets
- Subtitles: auto-load, add from file, style editor (font, size, colors, position)

### Library
- Start page with "Continue watching", recently played, collections and pinned folders
- Video and music folders, playlists, network streams and IPTV lists
- Plex: sign in, browse libraries, search, play and sync watch progress

### Android
- Touch layout: swipe for volume and brightness, double-tap the edges to skip
- Player bar you can rearrange live with drag and drop
- Screen lock, video scaling modes, Plex libraries in a swipe-in drawer
- Recognizes gesture navigation and keeps the back gesture working

### Windows
- Installer without admin rights, bundles FFmpeg, optional "Open with" entries
- Borderless window, always on top, keyboard shortcuts you can rebind

### General
- German and English, light and dark theme, your own accent color
- In-app updates from GitHub releases, with the changelog shown before installing
