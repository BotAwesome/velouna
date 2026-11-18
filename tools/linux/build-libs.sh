#!/bin/bash
# Builds the bundled Linux runtime of Velouna inside an Ubuntu 22.04 container
# (glibc 2.35: Pop!_OS 22.04 and newer):
#   lib/libmpv.so.2 + the libraries it needs that a desktop may lack,
#   ffmpeg, ffprobe (static), yt-dlp (standalone).
#
#   docker run --rm -v "$PWD/out:/out" -v "$PWD/build-libs.sh:/build-libs.sh:ro" ubuntu:22.04 bash /build-libs.sh
#
# mpv and FFmpeg come from mpv-build (current releases, FFmpeg and libass
# linked into libmpv). Result: /out/runtime/{lib,ffmpeg,ffprobe,yt-dlp}.
set -euo pipefail
export DEBIAN_FRONTEND=noninteractive
J=$(nproc)

apt-get update -qq
apt-get install -y -qq --no-install-recommends \
  ca-certificates curl git xz-utils patchelf file build-essential nasm pkg-config python3-pip ninja-build \
  autoconf automake libtool \
  libx11-dev libxext-dev libxrandr-dev libxinerama-dev libxss-dev libxpm-dev libxkbcommon-dev \
  libwayland-dev wayland-protocols libegl1-mesa-dev libgl1-mesa-dev libgbm-dev libdrm-dev \
  libva-dev libvdpau-dev libpulse-dev libasound2-dev libpipewire-0.3-dev \
  libfreetype6-dev libfribidi-dev libharfbuzz-dev libfontconfig1-dev libssl-dev \
  libjpeg-dev liblcms2-dev libuchardet-dev libluajit-5.1-dev zlib1g-dev libbz2-dev liblzma-dev >/dev/null
pip3 install -q 'meson>=1.3'

cd /root
git clone -q --depth 1 https://github.com/mpv-player/mpv-build.git
cd mpv-build
./use-mpv-release >/dev/null
./use-ffmpeg-release >/dev/null
# libmpv as a shared library, no mpv player binary, with Lua for the
# yt-dlp hook (links to video sites); FFmpeg with TLS for
# https streams, without NVIDIA CUDA probing (vaapi/vdpau stay). The same
# FFmpeg build also gives the ffmpeg and ffprobe programs (thumbnails,
# durations, converting), a few MB each instead of 170 MB static builds.
# No ALSA: Ubuntu 24.04's libasound2t64 lacks the ALSA_0.9 symbol versions
# a 22.04 build links against; sound goes through PulseAudio/PipeWire.
printf -- '-Dlibmpv=true\n-Dcplayer=false\n-Dlua=luajit\n-Dalsa=disabled\n-Djavascript=disabled\n-Dmanpage-build=disabled\n' > mpv_options
printf -- '--enable-version3\n--enable-openssl\n--disable-cuda-llvm\n--enable-ffmpeg\n--enable-ffprobe\n--disable-ffplay\n--disable-alsa\n--disable-doc\n' > ffmpeg_options
./rebuild -j"$J" >/root/build.log 2>&1 || { tail -60 /root/build.log; exit 1; }

R=/out/runtime
rm -rf "$R"; mkdir -p "$R/lib"
LIB=$(find mpv/build -maxdepth 1 -name 'libmpv.so.2*' -type f | head -1)
cp "$LIB" "$R/lib/libmpv.so.2"

# Libraries a desktop always has (graphics drivers, audio servers, X11,
# Wayland, glibc, fonts) stay out, like AppImage's excludelist; the rest
# is copied next to libmpv.
EXCLUDE='^(linux-vdso|ld-linux|libc\.|libm\.|libdl\.|libpthread\.|librt\.|libresolv\.|libutil\.|libgcc_s|libstdc\+\+|libGL|libEGL|libGLX|libGLdispatch|libOpenGL|libgbm|libdrm|libX|libxcb|libxkbcommon|libwayland|libasound|libpulse|libpipewire|libfontconfig|libfreetype|libharfbuzz|libexpat|libz\.|libdbus|libsystemd|libudev|libva|libvdpau|libffi|libglib|libgio|libgobject)'
copy_deps() {
  ldd "$1" | awk '/=> \//{print $1" "$3}' | while read -r name path; do
    if echo "$name" | grep -Eq "$EXCLUDE"; then continue; fi
    if [ ! -e "$R/lib/$name" ]; then
      cp -L "$path" "$R/lib/$name"
      copy_deps "$R/lib/$name"
    fi
  done
}
copy_deps "$R/lib/libmpv.so.2"
cp ffmpeg_build/ffmpeg ffmpeg_build/ffprobe "$R/"
copy_deps "$R/ffmpeg"
copy_deps "$R/ffprobe"
# ldd lists the whole tree, also what excluded system libraries (libpulse)
# pull in; keep only what our own files load through DT_NEEDED.
keep=" libmpv.so.2 "
walk() {
  for n in $(readelf -d "$1" | awk '/NEEDED/{gsub(/[][]/,"",$5); print $5}'); do
    if [ -e "$R/lib/$n" ] && ! echo "$keep" | grep -q " $n "; then keep="$keep$n "; walk "$R/lib/$n"; fi
  done
}
walk "$R/lib/libmpv.so.2"; walk "$R/ffmpeg"; walk "$R/ffprobe"
for f in "$R"/lib/*; do echo "$keep" | grep -q " $(basename "$f") " || rm "$f"; done
for f in "$R"/lib/*.so*; do patchelf --set-rpath '$ORIGIN' "$f"; done
patchelf --set-rpath '$ORIGIN/lib' "$R/ffmpeg" "$R/ffprobe"
strip --strip-unneeded "$R"/lib/*.so* "$R/ffmpeg" "$R/ffprobe" || true

# yt-dlp as one standalone file (links to video sites).
curl -fsSL -o "$R/yt-dlp" https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp_linux
chmod 755 "$R/ffmpeg" "$R/ffprobe" "$R/yt-dlp"
"$R/ffmpeg" -hide_banner -version | head -1

echo "== bundled"; ls -la "$R" "$R/lib"
echo "== libmpv needs from the system"; ldd "$R/lib/libmpv.so.2" | grep -v "$R" | awk '{print $1}' | sort | tr '\n' ' '; echo
echo "== glibc"; objdump -T "$R"/lib/*.so* | grep -o 'GLIBC_2\.[0-9]*' | sort -V | tail -1
cat mpv/build/meson-logs/meson-log.txt 2>/dev/null | grep -m1 "Project version" || true
chown -R "${OUT_UID:-0}:${OUT_GID:-0}" /out
echo DONE
