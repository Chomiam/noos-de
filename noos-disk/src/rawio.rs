//! Accès brut au disque (descripteur fourni par udisks) : test de vitesse, création et
//! restauration d'image disque. Tout se fait dans un fil à part ; la progression revient par
//! un canal.

use std::{
    io::{Read, Write},
    os::fd::{AsRawFd, OwnedFd},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Instant,
};

/// Progression : (octets traités, total, débit en octets/s).
pub type Progress = async_channel::Sender<(u64, u64, f64)>;

/// Tampon aligné (O_DIRECT exige des lectures alignées sur le secteur).
struct Aligned {
    ptr: *mut u8,
    len: usize,
    layout: std::alloc::Layout,
}

impl Aligned {
    fn new(len: usize) -> Self {
        let layout = std::alloc::Layout::from_size_align(len, 4096).unwrap();
        // SAFETY: taille non nulle, alignement valide.
        let ptr = unsafe { std::alloc::alloc(layout) };
        assert!(!ptr.is_null());
        Self { ptr, len, layout }
    }
}

impl Drop for Aligned {
    fn drop(&mut self) {
        // SAFETY: alloué avec ce même layout.
        unsafe { std::alloc::dealloc(self.ptr, self.layout) }
    }
}

fn pread(fd: &OwnedFd, buf: &mut Aligned, len: usize, offset: u64) -> std::io::Result<usize> {
    // SAFETY: tampon valide de `buf.len` octets, len <= buf.len.
    let n = unsafe { libc::pread(fd.as_raw_fd(), buf.ptr.cast(), len.min(buf.len), offset as libc::off_t) };
    if n < 0 { Err(std::io::Error::last_os_error()) } else { Ok(n as usize) }
}

#[derive(Debug, Clone, Default)]
pub struct Bench {
    /// Lecture séquentielle (octets/s) : moyenne, minimum, maximum.
    pub seq_avg: f64,
    pub seq_min: f64,
    pub seq_max: f64,
    /// Temps d'accès aléatoire moyen (ms).
    pub access_ms: f64,
    /// Débits mesurés le long du disque (position 0–1, octets/s).
    pub samples: Vec<(f64, f64)>,
}

/// Test de vitesse en lecture seule : 24 lectures de 32 Mio réparties sur tout le disque,
/// puis 120 accès aléatoires de 4 Kio.
pub fn benchmark(fd: OwnedFd, size: u64, progress: Progress, cancel: Arc<AtomicBool>) -> Result<Bench, String> {
    const CHUNK: usize = 32 << 20;
    const N: u64 = 24;
    let mut buf = Aligned::new(CHUNK);
    let mut out = Bench { seq_min: f64::MAX, ..Default::default() };
    let span = size.saturating_sub(CHUNK as u64);
    for k in 0..N {
        if cancel.load(Ordering::Relaxed) {
            return Err("annulé".into());
        }
        let off = (span / (N - 1) * k) & !4095;
        let t = Instant::now();
        let n = pread(&fd, &mut buf, CHUNK, off).map_err(|e| e.to_string())?;
        let rate = n as f64 / t.elapsed().as_secs_f64().max(1e-6);
        out.samples.push((k as f64 / (N - 1) as f64, rate));
        out.seq_min = out.seq_min.min(rate);
        out.seq_max = out.seq_max.max(rate);
        let _ = progress.send_blocking((k + 1, N + 4, rate));
    }
    out.seq_avg = out.samples.iter().map(|s| s.1).sum::<f64>() / out.samples.len().max(1) as f64;
    let mut seed: u64 = 0x9E3779B97F4A7C15;
    let mut total = 0.0;
    for i in 0..120u64 {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        let off = (seed % size.max(4096)) & !4095;
        let t = Instant::now();
        pread(&fd, &mut buf, 4096, off).map_err(|e| e.to_string())?;
        total += t.elapsed().as_secs_f64();
        if i % 30 == 29 {
            let _ = progress.send_blocking((N + 1 + i / 30, N + 4, out.seq_avg));
        }
    }
    out.access_ms = total / 120.0 * 1000.0;
    Ok(out)
}

/// Copie le disque (fd) vers un fichier image.
pub fn backup(fd: OwnedFd, size: u64, dest: &std::path::Path, progress: Progress, cancel: Arc<AtomicBool>) -> Result<(), String> {
    let mut src = std::fs::File::from(fd);
    let mut out = std::fs::File::create(dest).map_err(|e| e.to_string())?;
    copy(&mut src, &mut out, size, progress, cancel).inspect_err(|_| {
        let _ = std::fs::remove_file(dest);
    })
}

/// Écrit un fichier image sur le disque (fd).
pub fn restore(fd: OwnedFd, src: &std::path::Path, progress: Progress, cancel: Arc<AtomicBool>) -> Result<(), String> {
    let mut input = std::fs::File::open(src).map_err(|e| e.to_string())?;
    let size = input.metadata().map(|m| m.len()).unwrap_or(0);
    let mut out = std::fs::File::from(fd);
    copy(&mut input, &mut out, size, progress, cancel)?;
    out.sync_all().map_err(|e| e.to_string())
}

fn copy(src: &mut impl Read, dst: &mut impl Write, size: u64, progress: Progress, cancel: Arc<AtomicBool>) -> Result<(), String> {
    let mut buf = vec![0u8; 4 << 20];
    let (mut done, start, mut last) = (0u64, Instant::now(), Instant::now());
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err("annulé".into());
        }
        let n = src.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        dst.write_all(&buf[..n]).map_err(|e| e.to_string())?;
        done += n as u64;
        if last.elapsed().as_millis() > 250 {
            last = Instant::now();
            let _ = progress.send_blocking((done, size, done as f64 / start.elapsed().as_secs_f64().max(1e-3)));
        }
    }
    dst.flush().map_err(|e| e.to_string())?;
    let _ = progress.send_blocking((done, size.max(done), done as f64 / start.elapsed().as_secs_f64().max(1e-3)));
    Ok(())
}
