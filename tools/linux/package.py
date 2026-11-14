"""Builds the Linux packages of Velour from a cross-compiled binary.

    python tools/linux/package.py [path/to/velour]

Default binary: target/x86_64-unknown-linux-gnu/release/velour (built with
`cargo zigbuild --release -p velour --target x86_64-unknown-linux-gnu.2.31`,
so it runs on glibc 2.31+: Pop!_OS / Ubuntu 20.04 and newer).

Writes to dist/:
  velour_<v>_amd64.deb             Debian/Ubuntu/Pop!_OS/Mint (apt pulls libmpv)
  velour-<v>-linux-x86_64.tar.gz   any distro: unpack, ./install.sh

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
BIN = sys.argv[1] if len(sys.argv) > 1 else os.path.join(ROOT, "target", "x86_64-unknown-linux-gnu", "release", "velour")
DIST = os.path.join(ROOT, "dist")
NOW = int(time.time())

cargo = open(os.path.join(ROOT, "Cargo.toml"), encoding="utf-8").read()
VERSION = re.search(r'^version\s*=\s*"([^"]+)"', cargo, re.M).group(1)
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
Name=Velour
GenericName=Media Player
GenericName[de]=Medienwiedergabe
Comment=Videos, music, Plex, TV and radio
Comment[de]=Videos, Musik, Plex, TV und Radio
Exec=velour %F
Icon=velour
Terminal=false
Categories=AudioVideo;Video;Audio;Player;
Keywords=video;music;player;mpv;plex;tv;radio;
StartupWMClass=velour
MimeType={";".join(MIME)};
"""

INSTALL_SH = """#!/bin/sh
# Installs Velour for the current user (no root needed).
set -e
cd "$(dirname "$0")"
mkdir -p "$HOME/.local/bin" "$HOME/.local/share/applications" "$HOME/.local/share/icons/hicolor/scalable/apps"
install -m 755 velour "$HOME/.local/bin/velour"
sed "s|^Exec=velour|Exec=$HOME/.local/bin/velour|" velour.desktop > "$HOME/.local/share/applications/velour.desktop"
install -m 644 velour.svg "$HOME/.local/share/icons/hicolor/scalable/apps/velour.svg"
update-desktop-database "$HOME/.local/share/applications" 2>/dev/null || true
gtk-update-icon-cache -q "$HOME/.local/share/icons/hicolor" 2>/dev/null || true
if ! ldconfig -p 2>/dev/null | grep -q 'libmpv\\.so'; then
  echo "Velour needs libmpv:  sudo apt install libmpv2   (Fedora: sudo dnf install mpv-libs, Arch: sudo pacman -S mpv)"
fi
echo "Velour is installed. Start it from the app menu or with: ~/.local/bin/velour"
"""

UNINSTALL_SH = """#!/bin/sh
# Removes the per-user install (your settings in ~/.config/velour stay).
rm -f "$HOME/.local/bin/velour" "$HOME/.local/share/applications/velour.desktop" "$HOME/.local/share/icons/hicolor/scalable/apps/velour.svg"
update-desktop-database "$HOME/.local/share/applications" 2>/dev/null || true
echo "Velour is removed."
"""

README = f"""Velour {VERSION} for Linux (beta)

Needs libmpv:
  Pop!_OS / Ubuntu / Debian / Mint:  sudo apt install libmpv2   (22.04: libmpv1)
  Fedora:                            sudo dnf install mpv-libs
  Arch:                              sudo pacman -S mpv
Optional: ffmpeg (thumbnails, seek-bar previews), yt-dlp (YouTube links).

Install for your user:   ./install.sh
Remove:                  ./uninstall.sh
Or start it right here:  ./velour
"""


def tar_add(tf, name, data, mode=0o644):
    ti = tarfile.TarInfo(name)
    ti.size = len(data)
    ti.mode = mode
    ti.mtime = NOW
    ti.uid = ti.gid = 0
    ti.uname = ti.gname = "root"
    tf.addfile(ti, io.BytesIO(data))


def tar_dir(tf, name):
    ti = tarfile.TarInfo(name)
    ti.type = tarfile.DIRTYPE
    ti.mode = 0o755
    ti.mtime = NOW
    ti.uname = ti.gname = "root"
    tf.addfile(ti)


def targz(files, dirs=()):
    buf = io.BytesIO()
    with gzip.GzipFile(fileobj=buf, mode="wb", mtime=NOW) as gz:
        with tarfile.open(fileobj=gz, mode="w", format=tarfile.GNU_FORMAT) as tf:
            for d in dirs:
                tar_dir(tf, d)
            for name, data, mode in files:
                tar_add(tf, name, data, mode)
    return buf.getvalue()


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
        ("./usr/bin/velour", binary, 0o755),
        ("./usr/share/applications/velour.desktop", DESKTOP.encode(), 0o644),
        ("./usr/share/icons/hicolor/scalable/apps/velour.svg", ICON_SVG, 0o644),
        ("./usr/share/doc/velour/copyright", b"Velour - GPL-3.0-or-later\nhttps://github.com/BotAwesome/velour\n", 0o644),
    ]
    dirs = ["./", "./usr/", "./usr/bin/", "./usr/share/", "./usr/share/applications/", "./usr/share/icons/",
            "./usr/share/icons/hicolor/", "./usr/share/icons/hicolor/scalable/", "./usr/share/icons/hicolor/scalable/apps/",
            "./usr/share/doc/", "./usr/share/doc/velour/"]
    installed_kb = sum(len(d) for _, d, _ in data_files) // 1024 + 1
    control = f"""Package: velour
Version: {VERSION}
Architecture: amd64
Maintainer: BotAwesome <53971900+BotAwesome@users.noreply.github.com>
Installed-Size: {installed_kb}
Depends: libmpv2 | libmpv1
Recommends: ffmpeg
Suggests: yt-dlp
Section: video
Priority: optional
Homepage: https://botawesome.github.io/velour/
Description: Calm, fast media player built on mpv
 Videos and music, your Plex server and thousands of free TV and radio
 stations in one player with thin bars and a thick seek bar.
"""
    postinst = b"#!/bin/sh\nset -e\nupdate-desktop-database -q /usr/share/applications 2>/dev/null || true\ngtk-update-icon-cache -q /usr/share/icons/hicolor 2>/dev/null || true\n"
    control_tgz = targz([("./control", control.encode(), 0o644), ("./postinst", postinst, 0o755), ("./postrm", postinst, 0o755)], ["./"])
    data_tgz = targz(data_files, dirs)
    deb = ar([("debian-binary", b"2.0\n"), ("control.tar.gz", control_tgz), ("data.tar.gz", data_tgz)])
    deb_path = os.path.join(DIST, f"velour_{VERSION}_amd64.deb")
    open(deb_path, "wb").write(deb)

    # ---------------------------------------------------------------- tar.gz
    top = f"velour-{VERSION}-linux-x86_64"
    portable = targz([
        (f"{top}/velour", binary, 0o755),
        (f"{top}/velour.desktop", DESKTOP.encode(), 0o644),
        (f"{top}/velour.svg", ICON_SVG, 0o644),
        (f"{top}/install.sh", INSTALL_SH.encode(), 0o755),
        (f"{top}/uninstall.sh", UNINSTALL_SH.encode(), 0o755),
        (f"{top}/README.txt", README.encode(), 0o644),
    ], [f"{top}/"])
    tgz_path = os.path.join(DIST, f"{top}.tar.gz")
    open(tgz_path, "wb").write(portable)

    for p in (deb_path, tgz_path):
        print(f"{os.path.basename(p)}  {os.path.getsize(p) / 1e6:.1f} MB")


main()
