use std::{sync::atomic::{AtomicU32, Ordering}, time::Duration};

pub struct AtomicAverage {
    // first 8 bits = count, latter 24 bits = sum
    inner: AtomicU32
}

impl AtomicAverage {
    pub const fn new() -> Self {
        Self { inner: AtomicU32::new(0) }
    }

    fn _load(&self) -> (u8, u32) {
        let inner = self.inner.load(Ordering::Relaxed);
        (((inner & 0xff000000) >> 24) as u8, inner & 0x00ffffff)
    }

    pub fn add(&self, dur: Duration) {
        // adjust to not go over 24 bits
        let dur = dur.as_micros() as u32 & 0x00ffffff;
        let count = 1 << 24;
        
        self.inner.fetch_add(count | dur, Ordering::Relaxed);
    }

    pub fn get(&self) -> Duration {
        let (count, val) = self._load();
        let dur = Duration::from_micros(val as u64);

        dur / (count as u32).max(1)
    }
}