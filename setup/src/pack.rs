//! velour-pack <stub.exe> <out.exe> <version> <name>=<file>...
//!
//! Writes the setup stub followed by the compressed program files.

#[path = "payload.rs"]
#[allow(dead_code)]
mod payload;

use std::fs::File;
use std::io::{self, BufWriter, Write};

fn main() -> io::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() < 4 {
        eprintln!("usage: velour-pack <stub.exe> <out.exe> <version> <name>=<file>...");
        std::process::exit(2);
    }
    let (stub, out, version) = (&args[0], &args[1], &args[2]);
    let stub_bytes = std::fs::read(stub)?;
    let mut w = BufWriter::new(File::create(out)?);
    w.write_all(&stub_bytes)?;
    let offset = stub_bytes.len() as u64;

    let mut counter = Counter { inner: w, n: 0 };
    let mut unpacked = 0u64;
    {
        let mut enc = zstd::stream::write::Encoder::new(&mut counter, 19)?;
        let threads = std::thread::available_parallelism().map(|n| n.get() as u32).unwrap_or(4);
        enc.multithread(threads)?;
        enc.long_distance_matching(true)?;
        enc.window_log(payload::WINDOW_LOG)?;
        for spec in &args[3..] {
            let (name, path) = spec.split_once('=').expect("entry must be name=file");
            let data = std::fs::read(path)?;
            eprintln!("  {name}  ({:.1} MB)", data.len() as f64 / 1e6);
            enc.write_all(&(name.len() as u16).to_le_bytes())?;
            enc.write_all(name.as_bytes())?;
            enc.write_all(&(data.len() as u64).to_le_bytes())?;
            enc.write_all(&data)?;
            unpacked += data.len() as u64;
        }
        enc.write_all(&0u16.to_le_bytes())?;
        enc.finish()?;
    }
    let len = counter.n;
    let mut w = counter.inner;
    payload::write_footer(&mut w, &payload::Footer { offset, len, unpacked, version: version.clone() })?;
    w.flush()?;
    eprintln!("{out}: {:.1} MB ({:.1} MB unpacked)", (offset + len) as f64 / 1e6, unpacked as f64 / 1e6);
    Ok(())
}

struct Counter<W> {
    inner: W,
    n: u64,
}

impl<W: Write> Write for Counter<W> {
    fn write(&mut self, b: &[u8]) -> io::Result<usize> {
        let n = self.inner.write(b)?;
        self.n += n as u64;
        Ok(n)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}
