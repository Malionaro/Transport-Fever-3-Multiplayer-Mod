//! Reading the running game's memory safely: whether an address may be read
//! before the hook reads it on the game's thread (`crate::order`,
//! `crate::seeds`).
//!
//! Windows only for now, like the step gate. Elsewhere nothing is readable,
//! and the hook's native reads refuse (fail closed).
//!
//! [`readable`] asks the system every time (`VirtualQuery`, a system call:
//! about a microsecond, tens of microseconds inside Sandboxie, which hooks
//! system calls). The hot paths check through [`Readable`] instead, whose
//! per-thread cache remembers the regions found readable until
//! [`invalidate`], which the hook calls before every simulation update and
//! at every world change.

#![allow(unsafe_code)]

use std::{
    cell::RefCell,
    sync::atomic::{AtomicU64, Ordering},
};

/// The committed, readable region `[base, end)` that holds `address`, or
/// `None` when `address` is not readable.
#[cfg(windows)]
fn query(address: usize) -> Option<(usize, usize)> {
    use windows_sys::Win32::System::Memory::{
        MEM_COMMIT, MEMORY_BASIC_INFORMATION, PAGE_GUARD, PAGE_NOACCESS, VirtualQuery,
    };
    // SAFETY: MEMORY_BASIC_INFORMATION is plain data, zero is a valid value
    // for it, and VirtualQuery only writes into it.
    let mut info: MEMORY_BASIC_INFORMATION = unsafe { std::mem::zeroed() };
    let written = unsafe {
        VirtualQuery(
            address as *const _,
            &mut info,
            std::mem::size_of::<MEMORY_BASIC_INFORMATION>(),
        )
    };
    if written == 0 || info.State != MEM_COMMIT || info.Protect & (PAGE_NOACCESS | PAGE_GUARD) != 0
    {
        return None;
    }
    let base = info.BaseAddress as usize;
    Some((base, base.saturating_add(info.RegionSize)))
}

/// Nothing is known readable on this platform.
#[cfg(not(windows))]
fn query(_address: usize) -> Option<(usize, usize)> {
    None
}

/// Regions already found readable, most recent first; at most `N`.
#[derive(Debug, Clone)]
pub struct RegionCache<const N: usize> {
    regions: [(usize, usize); N],
    len: usize,
}

impl<const N: usize> RegionCache<N> {
    pub const fn new() -> Self {
        Self {
            regions: [(0, 0); N],
            len: 0,
        }
    }

    /// Whether `len` bytes at `address` are readable, asking `query` only
    /// for the parts no remembered region covers. `query(at)` answers the
    /// readable region `[base, end)` holding `at`, or `None`.
    pub fn readable_with(
        &mut self,
        address: usize,
        len: usize,
        mut query: impl FnMut(usize) -> Option<(usize, usize)>,
    ) -> bool {
        if len == 0 {
            return true;
        }
        let Some(end) = address.checked_add(len) else {
            return false;
        };
        let mut at = address;
        while at < end {
            if let Some(i) = self.regions[..self.len]
                .iter()
                .position(|(base, region_end)| *base <= at && at < *region_end)
            {
                let region_end = self.regions[i].1;
                // A hit moves to the front, so the regions in use stay
                // and the least recently used one goes first: evicting by
                // age alone thrashed once more than N regions were in play.
                self.regions[..=i].rotate_right(1);
                at = region_end;
                continue;
            }
            let Some((base, region_end)) = query(at) else {
                return false;
            };
            if base > at || region_end <= at {
                return false;
            }
            self.remember(base, region_end);
            at = region_end;
        }
        true
    }

    /// Puts `[base, end)` first, merged with every remembered region it
    /// overlaps or touches. VirtualQuery answers a region from the page of
    /// the address asked, not from where the region begins, so a check below
    /// a remembered region's start asks again and gets an overlapping answer:
    /// merged, it is one entry, not a second copy crowding out the others.
    fn remember(&mut self, mut base: usize, mut end: usize) {
        if N == 0 {
            return;
        }
        let mut kept = 0;
        for i in 0..self.len {
            let (other_base, other_end) = self.regions[i];
            if other_base <= end && base <= other_end {
                base = base.min(other_base);
                end = end.max(other_end);
            } else {
                self.regions[kept] = self.regions[i];
                kept += 1;
            }
        }
        let keep = kept.min(N - 1);
        self.regions.copy_within(0..keep, 1);
        self.regions[0] = (base, end);
        self.len = keep + 1;
    }

    /// How many regions are remembered.
    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
}

impl<const N: usize> Default for RegionCache<N> {
    fn default() -> Self {
        Self::new()
    }
}

/// A [`RegionCache`] that is good for one epoch: a check in a later epoch
/// forgets every region first.
#[derive(Debug, Clone)]
pub struct EpochCache<const N: usize> {
    epoch: u64,
    cache: RegionCache<N>,
}

impl<const N: usize> EpochCache<N> {
    pub const fn new() -> Self {
        Self {
            epoch: 0,
            cache: RegionCache::new(),
        }
    }

    /// As [`RegionCache::readable_with`], in `epoch`; also answers how many
    /// regions `query` was asked for (0: answered from the cache).
    pub fn readable_with(
        &mut self,
        epoch: u64,
        address: usize,
        len: usize,
        mut query: impl FnMut(usize) -> Option<(usize, usize)>,
    ) -> (bool, u32) {
        if epoch != self.epoch {
            self.epoch = epoch;
            self.cache = RegionCache::new();
        }
        let mut asked = 0;
        let readable = self.cache.readable_with(address, len, |at| {
            asked += 1;
            query(at)
        });
        (readable, asked)
    }
}

impl<const N: usize> Default for EpochCache<N> {
    fn default() -> Self {
        Self::new()
    }
}

/// The epoch of every thread's [`Readable`] cache. Starts at 1, so a fresh
/// cache (epoch 0) is always refilled.
static EPOCH: AtomicU64 = AtomicU64::new(1);
/// Checks answered from a cache, and checks that asked the system.
static HITS: AtomicU64 = AtomicU64::new(0);
static MISSES: AtomicU64 = AtomicU64::new(0);

thread_local! {
    static SHARED: RefCell<EpochCache<8>> = const { RefCell::new(EpochCache::new()) };
}

/// Forgets every region every thread's [`Readable`] cache remembers (they
/// refill on their next check). Called when memory may have been freed
/// that a cached region covered: before each simulation update and each
/// call of the game's step, when a world's GUI starts, when a load is asked
/// for.
pub fn invalidate() {
    EPOCH.fetch_add(1, Ordering::AcqRel);
}

/// The cache's hits and misses since the last take (the `perf:` line's).
pub fn take_counts() -> (u64, u64) {
    (
        HITS.swap(0, Ordering::Relaxed),
        MISSES.swap(0, Ordering::Relaxed),
    )
}

/// Readability checks through this thread's region cache: a region the
/// system said was committed and readable is remembered until the next
/// [`invalidate`], so the hot paths (the road fix's walk, the order fixes'
/// vectors, the game scripts' reseed) ask `VirtualQuery` once per region
/// and update, not once per word. Inside Sandboxie a `VirtualQuery` costs
/// tens of microseconds, so this is most of the hook's cost there.
///
/// Safe as long as nothing the engine frees in between is read: every
/// address the hook reads comes from a structure the engine keeps live, the
/// checks only guard against a layout the hook misreads, and the cache is
/// dropped at every update and world change, where the engine frees.
#[derive(Debug, Clone, Copy, Default)]
pub struct Readable;

impl Readable {
    pub const fn new() -> Self {
        Self
    }

    /// Whether `len` bytes at `address` are committed, readable memory.
    pub fn readable(&mut self, address: usize, len: usize) -> bool {
        if cfg!(not(windows)) {
            return false;
        }
        let epoch = EPOCH.load(Ordering::Acquire);
        let (readable, asked) =
            SHARED.with(|cache| cache.borrow_mut().readable_with(epoch, address, len, query));
        if asked == 0 {
            HITS.fetch_add(1, Ordering::Relaxed);
        } else {
            MISSES.fetch_add(1, Ordering::Relaxed);
        }
        readable
    }

    /// A plain value of the game's memory, only if it is readable.
    pub fn read<T: Copy>(&mut self, address: u64) -> Option<T> {
        let address = usize::try_from(address).ok()?;
        if !self.readable(address, std::mem::size_of::<T>()) {
            return None;
        }
        // SAFETY: `size_of::<T>()` bytes at `address` are committed,
        // readable memory, checked just above in this epoch; the read is
        // unaligned and by value.
        Some(unsafe { std::ptr::read_unaligned(address as *const T) })
    }
}

/// [`Readable::readable`], for one check.
pub fn readable_cached(address: usize, len: usize) -> bool {
    Readable.readable(address, len)
}

/// Whether `len` bytes at `address` are committed, readable memory: a fresh
/// question to the system every time.
#[cfg(windows)]
pub fn readable(address: usize, len: usize) -> bool {
    RegionCache::<0>::new().readable_with(address, len, query)
}

/// Whether `len` bytes at `address` may be read. Not known here, so `false`:
/// nothing native is read on this platform yet.
#[cfg(not(windows))]
pub fn readable(_address: usize, _len: usize) -> bool {
    false
}

#[cfg(test)]
mod cache_tests {
    use super::*;

    #[test]
    fn a_remembered_region_answers_without_asking_again() {
        let mut cache = RegionCache::<2>::new();
        let asked = std::cell::RefCell::new(Vec::new());
        let mut query = |at: usize| {
            asked.borrow_mut().push(at);
            // Readable: [0x1000, 0x3000) and [0x3000, 0x4000); not below.
            match at {
                0x1000..0x3000 => Some((0x1000, 0x3000)),
                0x3000..0x4000 => Some((0x3000, 0x4000)),
                _ => None,
            }
        };
        assert!(cache.readable_with(0x1010, 8, &mut query));
        assert!(cache.readable_with(0x2ff0, 0x10, &mut query));
        assert!(cache.readable_with(0x1000, 0x2000, &mut query));
        assert_eq!(*asked.borrow(), vec![0x1010], "one question for the region");
        // Across the region's end: the next region is asked for once.
        assert!(cache.readable_with(0x2ff8, 0x10, &mut query));
        assert!(cache.readable_with(0x3008, 8, &mut query));
        assert_eq!(*asked.borrow(), vec![0x1010, 0x3000]);
        // The two touch, and both are readable: one entry.
        assert_eq!(cache.len(), 1);
        // Past the readable memory: refused, whatever is remembered.
        assert!(!cache.readable_with(0x3ff8, 0x10, &mut query));
        assert!(!cache.readable_with(0x800, 8, &mut query));
        assert!(!cache.readable_with(usize::MAX - 4, 8, &mut query));
        assert!(cache.readable_with(0x10, 0, &mut query), "zero bytes");
    }

    /// Separate one-page regions (a gap between each, so none merge).
    fn pages(asked: &std::cell::Cell<usize>) -> impl FnMut(usize) -> Option<(usize, usize)> + '_ {
        move |at: usize| {
            asked.set(asked.get() + 1);
            let base = at & !0xfff;
            Some((base, base + 0x1000))
        }
    }

    #[test]
    fn the_least_recently_used_region_is_forgotten_first() {
        let mut cache = RegionCache::<2>::new();
        let asked = std::cell::Cell::new(0);
        let mut query = pages(&asked);
        for page in [0x1000, 0x3000, 0x5000] {
            assert!(cache.readable_with(page, 4, &mut query));
        }
        assert_eq!(cache.len(), 2);
        // 0x3000 and 0x5000 remembered; 0x1000 was forgotten.
        assert!(cache.readable_with(0x5004, 4, &mut query));
        assert!(cache.readable_with(0x3004, 4, &mut query));
        assert_eq!(asked.get(), 3);
        assert!(cache.readable_with(0x1004, 4, &mut query));
        assert_eq!(asked.get(), 4);
    }

    #[test]
    fn a_region_in_use_stays_while_others_come_and_go() {
        // One region read between visits to many others: evicting by age
        // alone asked for it again each time (the thrashing seen in the
        // road-entry benchmark); a hit keeps it.
        let mut cache = RegionCache::<2>::new();
        let asked = std::cell::Cell::new(0);
        let mut query = pages(&asked);
        assert!(cache.readable_with(0x1000, 4, &mut query));
        for other in [0x3000, 0x5000, 0x7000, 0x9000] {
            assert!(cache.readable_with(other, 4, &mut query));
            assert!(cache.readable_with(0x1004, 4, &mut query));
        }
        assert_eq!(asked.get(), 5, "0x1000 asked once, each other once");
    }

    #[test]
    fn a_check_below_a_remembered_start_merges_into_one_region() {
        // VirtualQuery answers from the page asked: [page, region end).
        let mut cache = RegionCache::<4>::new();
        let asked = std::cell::Cell::new(0);
        let mut query = |at: usize| {
            asked.set(asked.get() + 1);
            Some((at & !0xfff, 0x10_000))
        };
        assert!(cache.readable_with(0x8000, 4, &mut query));
        assert!(cache.readable_with(0x2000, 4, &mut query));
        assert_eq!(cache.len(), 1, "one region, not two overlapping copies");
        assert!(cache.readable_with(0x5000, 4, &mut query));
        assert!(cache.readable_with(0x9000, 4, &mut query));
        assert_eq!(asked.get(), 2);
    }

    #[test]
    fn a_cache_of_a_past_epoch_refuses_what_was_freed_since() {
        let mut cache = EpochCache::<4>::new();
        let mapped = std::cell::Cell::new(true);
        let query = |at: usize| {
            (mapped.get() && (0x1000..0x2000).contains(&at)).then_some((0x1000, 0x2000))
        };
        assert_eq!(cache.readable_with(1, 0x1800, 8, query), (true, 1));
        assert_eq!(cache.readable_with(1, 0x1808, 8, query), (true, 0), "a hit");
        // The world goes and its memory with it.
        mapped.set(false);
        assert_eq!(
            cache.readable_with(1, 0x1800, 8, query),
            (true, 0),
            "within the epoch the cache still answers: why every world change invalidates"
        );
        assert_eq!(
            cache.readable_with(2, 0x1800, 8, query),
            (false, 1),
            "the next epoch asks again, and the freed address is refused"
        );
    }

    #[test]
    fn a_region_that_does_not_hold_the_address_is_refused() {
        let mut cache = RegionCache::<4>::new();
        assert!(!cache.readable_with(0x1000, 4, |_| Some((0x2000, 0x3000))));
        assert!(!cache.readable_with(0x1000, 4, |_| Some((0x800, 0x1000))));
        assert!(cache.is_empty());
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    #[test]
    fn this_code_is_readable_and_unmapped_memory_is_not() {
        let here = this_code_is_readable_and_unmapped_memory_is_not as *const () as usize;
        assert!(readable(here, 16));
        // The null page is never mapped.
        assert!(!readable(0x10, 8));
        assert!(!readable(usize::MAX - 4, 8));
        assert!(readable(0x10, 0), "zero bytes are always readable");
        let mut probe = Readable::new();
        let value = 0x1234_5678_u32;
        assert_eq!(
            probe.read::<u32>(&value as *const u32 as u64),
            Some(0x1234_5678)
        );
        assert_eq!(probe.read::<u32>(0x10), None);
        assert!(readable_cached(here, 16));
    }

    #[test]
    fn a_freed_page_is_refused_once_the_cache_is_invalidated() {
        use windows_sys::Win32::System::Memory::{
            MEM_COMMIT, MEM_RELEASE, MEM_RESERVE, PAGE_READWRITE, VirtualAlloc, VirtualFree,
        };
        // SAFETY: a fresh page of our own.
        let page = unsafe {
            VirtualAlloc(
                std::ptr::null(),
                0x1000,
                MEM_COMMIT | MEM_RESERVE,
                PAGE_READWRITE,
            )
        } as usize;
        assert_ne!(page, 0);
        assert!(readable_cached(page, 8));
        assert!(readable_cached(page + 8, 8));
        // SAFETY: the page is ours and nothing refers to it any more.
        assert_ne!(unsafe { VirtualFree(page as *mut _, 0, MEM_RELEASE) }, 0);
        invalidate();
        assert!(!readable_cached(page, 8), "freed, and the cache forgot it");
        assert_eq!(Readable::new().read::<u64>(page as u64), None);
    }

    /// A check with and without the cache. Run with
    /// `cargo test --release -p tpf3mp-hook image::tests::readable_bench -- --ignored --nocapture`.
    #[test]
    #[ignore = "a benchmark: prints a readability check's cost with and without the cache"]
    fn readable_bench() {
        let words = vec![0u64; 64];
        let at = words.as_ptr() as usize;
        const N: usize = 200_000;
        let begin = std::time::Instant::now();
        for i in 0..N {
            assert!(readable(at + 8 * (i % 64), 8));
        }
        let fresh = begin.elapsed().as_nanos() as f64 / N as f64;
        invalidate();
        let begin = std::time::Instant::now();
        for i in 0..N {
            assert!(readable_cached(at + 8 * (i % 64), 8));
        }
        let cached = begin.elapsed().as_nanos() as f64 / N as f64;
        println!(
            "a readability check: {fresh:.0} ns asking VirtualQuery, {cached:.1} ns from the cache ({:.0}x)",
            fresh / cached
        );
    }
}
