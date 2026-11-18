"""Builds the Linux packages of Velouna from a cross-compiled binary.

    python tools/linux/package.py [path/to/velouna]

Default binary: target/x86_64-unknown-linux-gnu/release/velouna (built with
`cargo zigbuild --release -p velouna --target x86_64-unknown-linux-gnu.2.31`,
so it runs on glibc 2.31+: Pop!_OS / Ubuntu 20.04 and newer).

Bundles the runtime from dist/linux-runtime (built by tools/linux/build-libs.sh
on the VPS): lib/libmpv.so.2 with its libraries, ffmpeg, ffprobe, yt-dlp.

Writes to dist/:
  velouna_<v>_amd64.deb             Debian/Ubuntu/Pop!_OS/Mint, installs to /opt/velouna
  velouna-<v>-linux-x86_64.tar.gz   any distro: unpack, ./install.sh

Runs on Windows too: the .deb is assembled by hand (ar + tar), no dpkg needed.
"""
import gzip
import io
import os
import re
import sys
import tarfile
import time

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
BIN = sys.argv[1] if len(sys.argv) > 1 else os.path.join(ROOT, "target", "x86_64-unknown-linux-gnu", "release", "velouna")
DIST = os.path.join(ROOT, "dist")
RUNTIME = os.environ.get("VELOUNA_LINUX_RUNTIME", os.path.join(DIST, "linux-runtime"))
# System libraries the bundled libmpv and the window code (winit loads the
# X11 libraries at runtime) use: graphics, audio, fonts. Every
# desktop has them; the .deb names them so apt can check.
DEPENDS = "libc6 (>= 2.35), libegl1, libgl1, libgbm1, libdrm2, libx11-6, libx11-xcb1, libxcursor1, libxrandr2, libxi6, libxkbcommon0, libxkbcommon-x11-0, libwayland-client0, libwayland-egl1, libfontconfig1, libfreetype6, libharfbuzz0b, libpulse0, ca-certificates, libva2, libva-drm2, libva-x11-2, libvdpau1"
NOW = int(time.time())

cargo = open(os.path.join(ROOT, "Cargo.toml"), encoding="utf-8").read()
VERSION = re.search(r'^version\s*=\s*"([^"]+)"', cargo, re.M).group(1)
# Beta builds: VELOUNA_BETA=2 gives 0.1.3+beta2, which apt sorts above 0.1.3
# and the earlier betas, so "apt install ./…deb" replaces them.
if os.environ.get("VELOUNA_BETA"):
    VERSION = f"{VERSION}+beta{os.environ['VELOUNA_BETA']}"
ICON_SVG = open(os.path.join(ROOT, "docs", "assets", "icon.svg"), "rb").read()

# MIME types for "Open with": the formats src/media.rs plays.
MIME = [
    "video/mp4", "video/x-matroska", "video/webm", "video/x-msvideo", "video/quicktime", "video/mpeg",
    "video/x-flv", "video/ogg", "video/x-ms-wmv", "video/mp2t", "video/3gpp", "video/x-m4v", "video/vnd.rn-realvideo",
    "audio/mpeg", "audio/flac", "audio/x-flac", "audio/ogg", "audio/x-vorbis+ogg", "audio/x-opus+ogg", "audio/opus",
    "audio/wav", "audio/x-wav", "audio/mp4", "audio/x-m4a", "audio/aac", "audio/x-ms-wma", "audio/x-aiff",
    "audio/x-ape", "audio/x-wavpack", "audio/x-matroska", "audio/x-musepack",
    "audio/x-mpegurl", "application/vnd.apple.mpegurl", "audio/x-scpls", "application/xspf+xml", "application/x-cue",
]

DESKTOP = f"""[Desktop Entry]
Type=Application
Name=Velouna
GenericName=Media Player
GenericName[de]=Medienwiedergabe
Comment=Videos, music, Plex, TV and radio
Comment[de]=Videos, Musik, Plex, TV und Radio
Exec=velouna %F
Icon=velouna
Terminal=false
Categories=AudioVideo;Video;Audio;Player;
Keywords=video;music;player;mpv;plex;tv;radio;
StartupWMClass=velouna
MimeType={";".join(MIME)};
"""

# AppStream entry: lists Velouna in COSMIC Store / GNOME Software / Discover
# once the .deb is installed.
METAINFO = f"""<?xml version="1.0" encoding="UTF-8"?>
<component type="desktop-application">
  <id>io.github.botawesome.velouna</id>
  <name>Velouna</name>
  <summary>Calm, fast media player for videos, music, Plex, TV and radio</summary>
  <summary xml:lang="de">Ruhiger, schneller Player für Videos, Musik, Plex, TV und Radio</summary>
  <metadata_license>CC0-1.0</metadata_license>
  <project_license>GPL-3.0-or-later</project_license>
  <developer id="io.github.botawesome"><name>BotAwesome</name></developer>
  <description>
    <p>Videos and music, your Plex server and thousands of free TV and radio stations in one player with thin bars and a thick seek bar. Built on mpv.</p>
    <p xml:lang="de">Videos und Musik, dein Plex-Server und tausende freie TV- und Radiosender in einem Player mit schmalen Leisten und dicker Seek-Bar. Basiert auf mpv.</p>
  </description>
  <launchable type="desktop-id">velouna.desktop</launchable>
  <url type="homepage">https://botawesome.github.io/velouna/</url>
  <url type="vcs-browser">https://github.com/BotAwesome/velouna</url>
  <categories><category>AudioVideo</category><category>Video</category><category>Audio</category><category>Player</category></categories>
  <provides>{"".join(f"<mediatype>{m}</mediatype>" for m in MIME)}<binary>velouna</binary></provides>
  <content_rating type="oars-1.1"/>
  <releases><release version="{VERSION}" date="{time.strftime('%Y-%m-%d', time.gmtime(NOW))}"/></releases>
</component>
"""

# What the package ships besides Velouna, and under which licenses.
COPYRIGHT = """Velouna - GPL-3.0-or-later
https://github.com/BotAwesome/velouna

Bundled in /opt/velouna (Linux builds):
  lib/libmpv.so.2   mpv (GPL-2.0-or-later) with FFmpeg (built --enable-gpl
                    --enable-version3, so GPL-3.0-or-later), libass, LuaJIT
  ffmpeg, ffprobe   FFmpeg, same build (GPL-3.0-or-later)
  yt-dlp            yt-dlp standalone build (Unlicense)
  lib/*             OpenSSL 3 (Apache-2.0), FriBidi (LGPL-2.1+), liblzma
                    (0BSD/public domain), bzip2 (bzip2 license), Little CMS
                    (MIT), uchardet (MPL-1.1/GPL/LGPL), libjpeg-turbo (BSD-style),
                    LuaJIT (MIT) - taken from Ubuntu 22.04
Sources: https://mpv.io, https://ffmpeg.org, https://github.com/yt-dlp/yt-dlp
Build script: tools/linux/build-libs.sh in the Velouna sources.
"""

INSTALL_SH = """#!/bin/sh
# Installs Velouna for the current user (no root needed).
set -e
cd "$(dirname "$0")"
# The app was called Velour before: its per-user install goes (settings stay
# and are moved over on the first start).
rm -rf "$HOME/.local/share/velour/app"
rm -f "$HOME/.local/bin/velour" "$HOME/.local/share/applications/velour.desktop" "$HOME/.local/share/icons/hicolor/scalable/apps/velour.svg"
rmdir "$HOME/.local/share/velour" 2>/dev/null || true
APP="$HOME/.local/share/velouna/app"
mkdir -p "$HOME/.local/bin" "$HOME/.local/share/applications" "$HOME/.local/share/icons/hicolor/scalable/apps"
rm -rf "$APP"; mkdir -p "$APP"
cp -a velouna lib ffmpeg ffprobe yt-dlp "$APP/"
ln -sf "$APP/velouna" "$HOME/.local/bin/velouna"
sed "s|^Exec=velouna|Exec=$APP/velouna|" velouna.desktop > "$HOME/.local/share/applications/velouna.desktop"
install -m 644 velouna.svg "$HOME/.local/share/icons/hicolor/scalable/apps/velouna.svg"
update-desktop-database "$HOME/.local/share/applications" 2>/dev/null || true
gtk-update-icon-cache -q "$HOME/.local/share/icons/hicolor" 2>/dev/null || true
echo "Velouna is installed. Start it from the app menu or with: ~/.local/bin/velouna"
"""

UNINSTALL_SH = """#!/bin/sh
# Removes the per-user install (your settings in ~/.config/velouna stay).
rm -rf "$HOME/.local/share/velouna/app"
rm -f "$HOME/.local/bin/velouna" "$HOME/.local/share/applications/velouna.desktop" "$HOME/.local/share/icons/hicolor/scalable/apps/velouna.svg"
update-desktop-database "$HOME/.local/share/applications" 2>/dev/null || true
echo "Velouna is removed."
"""

README = f"""Velouna {VERSION} for Linux

Everything is included: mpv (libmpv), FFmpeg, ffprobe and yt-dlp.
Needs glibc 2.35+ (Pop!_OS / Ubuntu 22.04 and newer, Debian 12, Fedora 36+).
Log file: ~/.cache/velouna/velouna.log

Install for your user:   ./install.sh
Remove:                  ./uninstall.sh
Or start it right here:  ./velouna
"""


def tar_add(tf, name, data, mode=0o644):
    ti = tarfile.TarInfo(name)
    ti.size = len(data)
    ti.mode = mode
    ti.mtime = NOW
    ti.uid = ti.gid = 0
    ti.uname = ti.gname = "root"
    tf.addfile(ti, io.BytesIO(data))


def tar_link(tf, name, target):
    ti = tarfile.TarInfo(name)
    ti.type = tarfile.SYMTYPE
    ti.linkname = target
    ti.mode = 0o777
    ti.mtime = NOW
    ti.uname = ti.gname = "root"
    tf.addfile(ti)


def tar_dir(tf, name):
    ti = tarfile.TarInfo(name)
    ti.type = tarfile.DIRTYPE
    ti.mode = 0o755
    ti.mtime = NOW
    ti.uname = ti.gname = "root"
    tf.addfile(ti)


def targz(files, dirs=(), links=()):
    buf = io.BytesIO()
    with gzip.GzipFile(fileobj=buf, mode="wb", mtime=NOW, compresslevel=9) as gz:
        with tarfile.open(fileobj=gz, mode="w", format=tarfile.GNU_FORMAT) as tf:
            for d in dirs:
                tar_dir(tf, d)
            for name, data, mode in files:
                tar_add(tf, name, data, mode)
            for name, target in links:
                tar_link(tf, name, target)
    return buf.getvalue()


def runtime_files(prefix):
    """(path, data, mode) of the bundled runtime below `prefix`."""
    if not os.path.isfile(os.path.join(RUNTIME, "lib", "libmpv.so.2")):
        sys.exit(f"runtime missing: {RUNTIME} (run tools/linux/build-libs.sh on the VPS)")
    out = []
    for name in ("ffmpeg", "ffprobe", "yt-dlp"):
        out.append((f"{prefix}/{name}", open(os.path.join(RUNTIME, name), "rb").read(), 0o755))
    for name in sorted(os.listdir(os.path.join(RUNTIME, "lib"))):
        out.append((f"{prefix}/lib/{name}", open(os.path.join(RUNTIME, "lib", name), "rb").read(), 0o644))
    return out


def ar(members):
    out = bytearray(b"!<arch>\n")
    for name, data in members:
        hdr = f"{name:<16}{NOW:<12}{0:<6}{0:<6}{'100644':<8}{len(data):<10}`\n".encode()
        assert len(hdr) == 60
        out += hdr + data
        if len(data) % 2:
            out += b"\n"
    return bytes(out)


def main():
    if not os.path.isfile(BIN):
        sys.exit(f"binary missing: {BIN}")
    binary = open(BIN, "rb").read()
    if binary[:4] != b"\x7fELF":
        sys.exit(f"not a Linux binary: {BIN}")
    os.makedirs(DIST, exist_ok=True)

    # ---------------------------------------------------------------- .deb
    data_files = [
        ("./opt/velouna/velouna", binary, 0o755),
        *runtime_files("./opt/velouna"),
        ("./usr/share/applications/velouna.desktop", DESKTOP.encode(), 0o644),
        ("./usr/share/icons/hicolor/scalable/apps/velouna.svg", ICON_SVG, 0o644),
        ("./usr/share/doc/velouna/copyright", COPYRIGHT.encode(), 0o644),
        ("./usr/share/metainfo/io.github.botawesome.velouna.metainfo.xml", METAINFO.encode(), 0o644),
    ]
    dirs = ["./", "./opt/", "./opt/velouna/", "./opt/velouna/lib/", "./usr/", "./usr/bin/", "./usr/share/", "./usr/share/applications/", "./usr/share/icons/",
            "./usr/share/icons/hicolor/", "./usr/share/icons/hicolor/scalable/", "./usr/share/icons/hicolor/scalable/apps/",
            "./usr/share/doc/", "./usr/share/doc/velouna/", "./usr/share/metainfo/"]
    installed_kb = sum(len(d) for _, d, _ in data_files) // 1024 + 1
    control = f"""Package: velouna
Version: {VERSION}
Architecture: amd64
Maintainer: BotAwesome <53971900+BotAwesome@users.noreply.github.com>
Installed-Size: {installed_kb}
Depends: {DEPENDS}
Conflicts: velour
Replaces: velour
Section: video
Priority: optional
Homepage: https://botawesome.github.io/velouna/
Description: Calm, fast media player built on mpv
 Videos and music, your Plex server and thousands of free TV and radio
 stations in one player with thin bars and a thick seek bar.
"""
    postinst = b"#!/bin/sh\nset -e\nupdate-desktop-database -q /usr/share/applications 2>/dev/null || true\ngtk-update-icon-cache -q /usr/share/icons/hicolor 2>/dev/null || true\nappstreamcli refresh-cache >/dev/null 2>&1 || true\n"
    control_tgz = targz([("./control", control.encode(), 0o644), ("./postinst", postinst, 0o755), ("./postrm", postinst, 0o755)], ["./"])
    data_tgz = targz(data_files, dirs, [("./usr/bin/velouna", "/opt/velouna/velouna")])
    deb = ar([("debian-binary", b"2.0\n"), ("control.tar.gz", control_tgz), ("data.tar.gz", data_tgz)])
    deb_path = os.path.join(DIST, f"velouna_{VERSION}_amd64.deb")
    open(deb_path, "wb").write(deb)

    # ---------------------------------------------------------------- tar.gz
    top = f"velouna-{VERSION}-linux-x86_64"
    portable = targz([
        (f"{top}/velouna", binary, 0o755),
        *runtime_files(top),
        (f"{top}/velouna.desktop", DESKTOP.encode(), 0o644),
        (f"{top}/velouna.svg", ICON_SVG, 0o644),
        (f"{top}/COPYING-bundled.txt", COPYRIGHT.encode(), 0o644),
        (f"{top}/install.sh", INSTALL_SH.encode(), 0o755),
        (f"{top}/uninstall.sh", UNINSTALL_SH.encode(), 0o755),
        (f"{top}/README.txt", README.encode(), 0o644),
    ], [f"{top}/", f"{top}/lib/"])
    tgz_path = os.path.join(DIST, f"{top}.tar.gz")
    open(tgz_path, "wb").write(portable)

    for p in (deb_path, tgz_path):
        print(f"{os.path.basename(p)}  {os.path.getsize(p) / 1e6:.1f} MB")


main()
