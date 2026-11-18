//! Size limit for the on-disk cache: library thumbnails, song analyses and
//! downloaded artwork. Files are stamped when used; past the limit the
//! longest unused ones go first. Runs once at start, in the background.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime};

/// Folders under the cache dir that may be trimmed (everything in them can
/// be made again).
const DIRS: &[&str] = &["thumbs", "analysis", "plex_img", "logo_img"];

/// Files from older Velouna versions, removed on sight.
const LEGACY: &[(&str, &str)] = &[("thumbs", "rgba"), ("analysis", "json")];

/// Leftovers of writes that never finished (only when older than this, so a
/// write in progress is left alone).
const STALE: Duration = Duration::from_secs(3600);

/// Marks a cache file as just used.
pub fn touch(p: &Path) {
    if let Ok(f) = std::fs::File::options().write(true).open(p) {
        let _ = f.set_modified(SystemTime::now());
    }
}

/// Trims the cache in a thread; `used` gets the size afterwards.
pub fn trim_in_background(limit_mb: u32, used: Arc<AtomicU64>) {
    let _ = std::thread::Builder::new().name("cache trim".into()).spawn(move || {
        let t0 = std::time::Instant::now();
        let (total, removed) = trim(&crate::config::cache_dir(), limit_mb as u64 * 1024 * 1024);
        used.store(total, Ordering::Relaxed);
        log::info!("cache: {} MB in use, {removed} files removed, {} ms", total / 1_048_576, t0.elapsed().as_millis());
    });
}

/// Removes legacy and stale files, then the longest unused ones until the
/// rest fits in 90 % of `limit`. Returns (bytes left, files removed).
pub fn trim(root: &Path, limit: u64) -> (u64, usize) {
    let now = SystemTime::now();
    let mut files: Vec<(PathBuf, u64, SystemTime)> = Vec::new();
    let mut removed = 0;
    for dir in DIRS {
        let mut stack = vec![root.join(dir)];
        while let Some(d) = stack.pop() {
            let Ok(rd) = std::fs::read_dir(&d) else { continue };
            for e in rd.flatten() {
                let Ok(meta) = e.metadata() else { continue };
                let p = e.path();
                if meta.is_dir() {
                    stack.push(p);
                    continue;
                }
                let ext = p.extension().and_then(|x| x.to_str()).unwrap_or("").to_ascii_lowercase();
                let mtime = meta.modified().unwrap_or(now);
                let old = now.duration_since(mtime).map(|a| a > STALE).unwrap_or(false);
                let legacy = LEGACY.iter().any(|(d, x)| d == dir && *x == ext);
                let stale = old && matches!(ext.as_str(), "tmp" | "pcm");
                if legacy || stale {
                    if std::fs::remove_file(&p).is_ok() {
                        removed += 1;
                    }
                    continue;
                }
                files.push((p, meta.len(), mtime));
            }
        }
    }
    let mut total: u64 = files.iter().map(|f| f.1).sum();
    if total > limit {
        files.sort_by_key(|f| f.2);
        let target = limit / 10 * 9;
        for (p, len, _) in files {
            if total <= target {
                break;
            }
            if std::fs::remove_file(&p).is_ok() {
                total -= len;
                removed += 1;
            }
        }
    }
    (total, removed)
}

#[cfg(test)]
mod tests {
    #[test]
    fn trims_oldest_first() {
        let root = std::env::temp_dir().join(format!("velouna-cache-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let thumbs = root.join("thumbs");
        std::fs::create_dir_all(&thumbs).unwrap();
        let old = std::time::SystemTime::now() - std::time::Duration::from_secs(7200);
        for i in 0..10 {
            let p = thumbs.join(format!("{i}.jpg"));
            std::fs::write(&p, vec![0u8; 1000]).unwrap();
            // 0 is the oldest, 9 the newest
            let f = std::fs::File::options().write(true).open(&p).unwrap();
            f.set_modified(old + std::time::Duration::from_secs(i * 60)).unwrap();
        }
        std::fs::write(thumbs.join("legacy.rgba"), b"x").unwrap();
        std::fs::write(root.join("probe_cache.json"), b"{}").unwrap();
        let (left, removed) = super::trim(&root, 5000);
        assert_eq!(left, 4000);
        assert_eq!(removed, 7); // legacy file plus 0..=5
        assert!(!thumbs.join("5.jpg").exists() && thumbs.join("6.jpg").exists());
        assert!(root.join("probe_cache.json").exists());
        let _ = std::fs::remove_dir_all(&root);
    }
}
