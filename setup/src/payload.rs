//! The program files travel inside the setup exe:
//!
//! ```text
//! [setup stub exe][zstd stream of entries][footer]
//! entry  = u16 name_len | name (utf-8, '/' separated) | u64 size | bytes
//!          (a name_len of 0 ends the list)
//! footer = u64 offset | u64 len | u64 unpacked | 16 bytes version | 8 bytes MAGIC
//! ```
//!
//! An exe without footer is the bare stub, which is what gets installed as
//! `uninstall.exe`.

use std::fs::File;
use std::io::{self, BufReader, Read, Seek, SeekFrom, Write};
use std::path::Path;

pub const MAGIC: &[u8; 8] = b"VLRPACK1";
pub const FOOTER_LEN: u64 = 8 * 3 + 16 + 8;
/// zstd window: big enough that ffprobe.exe compresses against ffmpeg.exe.
pub const WINDOW_LOG: u32 = 28;

#[derive(Clone, Debug)]
pub struct Footer {
    pub offset: u64,
    pub len: u64,
    pub unpacked: u64,
    pub version: String,
}

pub fn read_footer(exe: &Path) -> io::Result<Option<Footer>> {
    let mut f = File::open(exe)?;
    let size = f.metadata()?.len();
    if size < FOOTER_LEN {
        return Ok(None);
    }
    f.seek(SeekFrom::Start(size - FOOTER_LEN))?;
    let mut b = [0u8; FOOTER_LEN as usize];
    f.read_exact(&mut b)?;
    if &b[40..48] != MAGIC {
        return Ok(None);
    }
    let u = |i: usize| u64::from_le_bytes(b[i..i + 8].try_into().unwrap());
    let version = String::from_utf8_lossy(&b[24..40]).trim_end_matches('\0').to_string();
    Ok(Some(Footer { offset: u(0), len: u(8), unpacked: u(16), version }))
}

pub fn write_footer(w: &mut impl Write, f: &Footer) -> io::Result<()> {
    w.write_all(&f.offset.to_le_bytes())?;
    w.write_all(&f.len.to_le_bytes())?;
    w.write_all(&f.unpacked.to_le_bytes())?;
    let mut v = [0u8; 16];
    let vb = f.version.as_bytes();
    v[..vb.len().min(16)].copy_from_slice(&vb[..vb.len().min(16)]);
    w.write_all(&v)?;
    w.write_all(MAGIC)
}

/// Streams the entries out of `exe`. `open` returns the writer for an entry
/// (by relative name); `progress` gets the bytes written so far.
pub fn unpack(
    exe: &Path,
    footer: &Footer,
    mut open: impl FnMut(&str) -> io::Result<Box<dyn Write>>,
    mut progress: impl FnMut(u64),
) -> io::Result<()> {
    let mut f = File::open(exe)?;
    f.seek(SeekFrom::Start(footer.offset))?;
    let mut dec = zstd::stream::read::Decoder::new(BufReader::new(f).take(footer.len))?;
    dec.window_log_max(WINDOW_LOG)?;
    let mut done = 0u64;
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let mut l = [0u8; 2];
        dec.read_exact(&mut l)?;
        let name_len = u16::from_le_bytes(l) as usize;
        if name_len == 0 {
            return Ok(());
        }
        let mut name = vec![0u8; name_len];
        dec.read_exact(&mut name)?;
        let name = String::from_utf8(name).map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "bad entry name"))?;
        if name.contains("..") || name.starts_with('/') || name.contains(':') {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "bad entry name"));
        }
        let mut s = [0u8; 8];
        dec.read_exact(&mut s)?;
        let mut left = u64::from_le_bytes(s);
        let mut out = open(&name)?;
        while left > 0 {
            let n = (left as usize).min(buf.len());
            dec.read_exact(&mut buf[..n])?;
            out.write_all(&buf[..n])?;
            left -= n as u64;
            done += n as u64;
            progress(done);
        }
        out.flush()?;
    }
}
