# Changelog

All notable changes to Velouna (called Velour up to 0.2.1). The newest version is at the top; each
section becomes the text of its GitHub release and the "What's new" dialog.

## [0.6.0] - 2026-11-24

### HDR on Windows
- With "Use HDR" on in Windows, HDR videos play in real HDR with their full
  brightness and colors. Turn it off in the settings to always get SDR
- Switching Windows HDR on or off while a video plays is picked up right away
- The media info shows whether a video is shown in HDR
- HDR videos on SDR screens look better: brighter highlights, less washed-out colors

### Downloads
- Right-click a movie, episode or song from Plex, Jellyfin or Emby and pick
  "Download" – or "Download all" on a season, show, album or playlist
- Velouna downloads the original file, without Plex Pass or the server's own sync
- Downloaded items play from disk, and your progress still goes to the server.
  Cards show a small download mark; right-click again to delete the copy
- An interrupted download continues where it stopped

### Crash reports
- After a crash, Velouna shows the crash report on the next start and sends it only
  if you click "Send". It has no identifier and nothing about you

## [0.5.0] - 2026-11-24

### Controls that fit what plays
- Music no longer shows tools for pictures (subtitles, the video menu, screenshots,
  picture-in-picture), and live TV and radio no longer show tools for files (repeat,
  shuffle, speed, bookmarks, converting). "Record stream" only appears for streams
- Songs always start from the beginning and show no progress. Audio of 20 minutes or
  more, like podcasts and audiobooks, continues where you left off, also from Plex
- A click that closes a menu no longer pauses the video behind it
- Menus with submenus are no longer stretched wide

### Chapters
- The new chapters button in the player bar opens a strip with a picture for every
  chapter. It only shows for files that have chapters
- Picking a chapter starts playing right away, also when paused

### Covers and pictures
- Songs show their own cover in the queue and on the now-playing page, read straight
  from the file. Songs without one get the Velouna cover
- Plex: seeking shows the server's own preview pictures
- Plugin sources appear in "Continue watching" and pick up where you left off

### Windows
- Media keys and the Windows media overlay show and control what plays: title,
  artist, cover, position, play and pause
- Previous, play/pause and next in the taskbar preview, and the progress of
  converting in the taskbar icon
- A right click on the taskbar icon lists "Continue watching" and recently played
- Thumbnails and song details come from Windows itself, which fills large folders
  much faster
- Uses less power while minimized
- The installer offers to make Velouna your default player

### Android
- Music plays with a fixed player bar at the bottom, without video gestures
- Videos get thumbnails and a preview while seeking, songs their title, artist and
  album, without needing FFmpeg on the phone
- The first setup fits on small screens

## [0.4.0] - 2026-11-18

### Plugins that do things
- Lua plugins: small scripts that react to what plays, add entries to the player menu and
  the main menu, buttons to the player bar, and their own settings page
- Sources from plugins: a plugin can bring podcasts, channels or anything else into the
  library, with folders, search and pictures, and its own row on the start page
- Segments: plugins can mark parts of a video. Velouna skips them, offers a "Skip" button,
  mutes them or just shows them in colour on the timeline. Every skip can be undone
- Plugins can clean up titles, change what is about to play, react when a stream fails,
  and add picture effects and sound filters
- Before a plugin is switched on, Velouna shows what it wants to do (for example talk to
  devices in your home network) and asks you first
- Every Lua plugin has a console: what it wrote, its errors, how long it took, and a prompt
  to try things out. `velouna --no-plugins` starts without any plugins
- Eight new examples to learn from: track log, clean titles, segments from a file, notes,
  effects, webhook, a shared module and podcasts from RSS. They and the documentation
  live in github.com/BotAwesome/velouna-example-plugins, which you can add under Repos

## [0.3.0] - 2026-11-18

### Velour is now Velouna
- New name, same player: other players were already called Velour. Settings, library and
  history move over by themselves on Windows and Linux, and the update from Velour installs
  Velouna in its place
- Android: Velouna is a new app next to the old one (app.velouna). Uninstall Velour once
  Velouna is set up
- The project lives at github.com/BotAwesome/velouna now. Old links lead there

### Music you can see
- Visualizers: pictures that follow the song. Velouna analyses each song once (beats, kicks,
  snares, build-ups and drops) and the picture reacts to them, also after seeking
- The Velounizer comes with three presets: Ink, a slowly running fluid of colour; Aurora;
  and Contour. Each with palettes, a seed and its own sliders
- The visualizer button in the player bar switches it on and off; its arrow picks the
  preset, palette and settings
- Songs without a visualizer get a now-playing page: cover, title, the queue of the album
  or folder, and details when there is room

### Plugins
- Settings → Extensions: install, switch on and off, update and remove plugins
- Catalog: plugins from repos, with the official Velouna repo already added. Tick what you
  want and install it in one go
- Repos: add anyone's repo by its link, or just one plugin of it by linking its folder.
  The Repos tab shows which updates are waiting
- Every repo is signed. Velouna remembers its key and blocks updates if it ever changes,
  with a clear warning and a 30-second wait before a new key can be accepted
- Own repo: pack your plugin folders into a signed repo, ready to put on GitHub
- Write your own visualizer with a GLSL shader and, if you like, a small Lua script. Reload
  picks up changed files right away. Docs and examples:
  https://github.com/BotAwesome/velouna-plugins

### Smaller things
- The cache (thumbnails, song analyses, artwork) stays below a size you choose in
  Settings → Privacy, 1 GB by default. The longest unused files go first
- mpv's own warnings and errors are written to Velouna's log

## [0.2.1] - 2026-11-17

### Library
- With the mini player showing, the tiles of a folder no longer slide up over the path and
  the toolbar. The pages of Plex, Jellyfin and Emby had the same fault

## [0.2.0] - 2026-11-17

### Jellyfin and Emby
- Connect a Jellyfin or Emby server with its address, user name and password, next to Plex or
  without it: start page with continue watching and next up, libraries, collections,
  playlists, search, and playback with your progress synced to the server
- If the server signs Velour out (for example after the device was removed there), Velour
  asks you to sign in again instead of showing an error
- The settings page "Plex" is now "Media servers" and lists all your servers

### Blazing fast
- From double-click to picture in about half a second, even when Velour was not running:
  a file opened at start now plays right away
- mpv 0.42 on Windows and Android, with FFmpeg 9 on Android
- Android: HTTPS streams no longer stall at the start

### Sidebar
- The first setup asks what the sidebar shows, with presets for your own files, media servers
  or TV and radio. "Settings → Interface → Sidebar" changes it later
- The Plex, Jellyfin and Emby groups in the sidebar fold in and out

### Player
- The repeat button (off, all, one file) is in the player bar by default, next to the
  settings. Bars you arranged yourself stay as they are

### Library next to the player
- Keeps the width you drag it to, also after a restart
- Has its own grid and list view (grid shows small tiles)
- Dragging files to the queue shows what you are carrying
- Files without a preview, like songs, get a box of the same size, so all titles line up

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
