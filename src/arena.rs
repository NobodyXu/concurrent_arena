use super::arcs::Arcs;
use super::bucket::Bucket;
use super::thread_id::get_thread_id;
use super::Arc;
use super::ArenaArc;

use core::cmp::min;

use const_fn_assert::{cfn_assert, cfn_assert_eq, cfn_assert_ne};

/// * `LEN` - Number of elements stored per bucket.
///    Must be less than or equal to `u32::MAX`, divisible by
///   `usize::BITS` and it must not be `0`.
/// * `BITARRAY_LEN` - Number bits in the bitmap per bucket.
///   Must be equal to `LEN / usize::BITS`.
///
///   For best performance, try to set this to number of CPUs that are going
///   to access `Arena` concurrently.
///
/// `Arena` stores the elements in buckets to ensure that the address
/// for elements are stable while improving efficiency.
///
/// Every bucket is of size `LEN`.
///
/// The larger `LEN` is, the more compact the `Arena` will be, however it might
/// also waste space if it is unused.
///
/// And, allocating a large chunk of memory takes more time.
///
/// `Arena` internally stores the array of buckets as a `triomphe::ThinArc`
/// and use `ArcSwapAny` to grow the array atomically, without blocking any
/// reader.
///
/// # Examples
///
/// If you provides `Arena` with invalid `LEM` or `BITARRAY_LEN`, then your
/// code will panic at runtime:
///
/// ```rust,should_panic
/// use concurrent_arena::*;
/// let arena = Arena::<u32, 1, 100>::new();
/// ```
///
/// To make it a compile time failure, you need to call
/// `max_buckets`:
///
/// ```rust,compile_fail
/// use concurrent_arena::*;
/// const MAX_BUCKETS: u32 = Arena::<u32, 1, 100>::max_buckets();
/// ```
#[derive(Debug)]
pub struct Arena<T, const BITARRAY_LEN: usize, const LEN: usize> {
    buckets: Arcs<Arc<Bucket<T, BITARRAY_LEN, LEN>>>,
}

impl<T: Sync + Send, const BITARRAY_LEN: usize, const LEN: usize> Default
    for Arena<T, BITARRAY_LEN, LEN>
{
    fn default() -> Self {
        Self::new()
    }
}

const fn check_const_generics<const BITARRAY_LEN: usize, const LEN: usize>() {
    let bits = usize::BITS as usize;

    cfn_assert!(LEN <= (u32::MAX as usize));
    cfn_assert_eq!(LEN % bits, 0);
    cfn_assert_ne!(LEN, 0);

    cfn_assert_eq!(LEN / bits, BITARRAY_LEN);
}

impl<T, const BITARRAY_LEN: usize, const LEN: usize> Arena<T, BITARRAY_LEN, LEN> {
    /// Maximum buckets `Arena` can have.
    pub const fn max_buckets() -> u32 {
        check_const_generics::<BITARRAY_LEN, LEN>();

        u32::MAX / (LEN as u32)
    }
}

impl<T: Send + Sync, const BITARRAY_LEN: usize, const LEN: usize> Arena<T, BITARRAY_LEN, LEN> {
    /// Would preallocate 2 buckets.
    pub fn new() -> Self {
        Self::with_capacity(2)
    }

    pub fn with_capacity(cap: u32) -> Self {
        check_const_generics::<BITARRAY_LEN, LEN>();

        let cap = min(cap, Self::max_buckets());
        let buckets = Arcs::new();

        buckets.grow(cap as usize, Arc::default);

        Self { buckets }
    }

    /// Return Ok(arc) on success, or Err((value, len)) where value is
    /// the input param `value` and `len` is the length of the `Arena` at the time
    /// of insertion.
    ///
    /// This function is lock-free.
    pub fn try_insert(&self, mut value: T) -> Result<ArenaArc<T, BITARRAY_LEN, LEN>, (T, u32)> {
        let slice = self.buckets.as_slice();
        let len = slice.len();

        debug_assert!(len <= Self::max_buckets() as usize);

        if len == 0 {
            return Err((value, 0));
        }

        let mut pos = get_thread_id() % len;

        let slice1_iter = slice[pos..].iter();
        let slice2_iter = slice[..pos].iter();

        for bucket in slice1_iter.chain(slice2_iter) {
            match Bucket::try_insert(bucket, pos as u32, value) {
                Ok(arc) => return Ok(arc),
                Err(val) => value = val,
            }

            pos = (pos + 1) % len;
        }

        Err((value, len as u32))
    }

    /// Try to reserve `min(new_len, Self::max_buckets())` buckets.
    ///
    /// This function is technically lock-free.
    pub fn try_reserve(&self, new_len: u32) -> bool {
        if new_len == 0 {
            return true;
        }

        let new_len = min(new_len, Self::max_buckets());
        self.buckets
            .try_grow(new_len as usize, Arc::default)
            .is_ok()
    }

    /// Reserve `min(new_len, Self::max_buckets())` buckets.
    pub fn reserve(&self, new_len: u32) {
        if new_len != 0 {
            let new_len = min(new_len, Self::max_buckets());
            self.buckets.grow(new_len as usize, Arc::default)
        }
    }

    /// Insert one value.
    ///
    /// If there isn't enough buckets, then try to reserve one bucket and
    /// restart the operation.
    pub fn insert(&self, mut value: T) -> ArenaArc<T, BITARRAY_LEN, LEN> {
        // Fast path where `try_reserve` is used to avoid locking.
        for _ in 0..5 {
            match self.try_insert(value) {
                Ok(arc) => return arc,
                Err((val, len)) => {
                    value = val;

                    // If len == Self::max_buckets(), then we would have to
                    // wait for slots to be removed from `Arena`.
                    if len != Self::max_buckets() {
                        // If try_reserve succeeds, then another new bucket is available.
                        //
                        // If try_reserve fail, then another thread is doing the
                        // reservation.
                        //
                        // We can simply restart operation, waiting for it to be done.
                        self.try_reserve(len + 4);
                    }
                }
            }
        }

        // Slow path where `reserve` is used.
        loop {
            match self.try_insert(value) {
                Ok(arc) => break arc,
                Err((val, len)) => {
                    value = val;

                    // If len == Self::max_buckets(), then we would have to
                    // wait for slots to be removed from `Arena`.
                    if len != Self::max_buckets() {
                        self.reserve(len + 8);
                    }
                }
            }
        }
    }

    /// May enter busy loop if the slot is not fully initialized.
    ///
    /// This function is lock free.
    pub fn remove(&self, slot: u32) -> Option<ArenaArc<T, BITARRAY_LEN, LEN>> {
        let bucket_index = slot / (LEN as u32);
        let index = slot % (LEN as u32);

        let bucket = self.buckets.as_slice()[bucket_index as usize].clone();

        Bucket::remove(bucket, bucket_index, index)
    }

    /// May enter busy loop if the slot is not fully initialized.
    ///
    /// This function is lock free.
    pub fn get(&self, slot: u32) -> Option<ArenaArc<T, BITARRAY_LEN, LEN>> {
        let bucket_index = slot / (LEN as u32);
        let index = slot % (LEN as u32);

        let bucket = self.buckets.as_slice()[bucket_index as usize].clone();

        Bucket::get(bucket, bucket_index, index)
    }

    /// Return number of buckets allocated.
    ///
    /// This function is lock free.
    pub fn len(&self) -> u32 {
        self.buckets.len() as u32
    }

    /// This function is lock free.
    pub fn is_empty(&self) -> bool {
        self.buckets.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use crate::*;

    #[test]
    fn test_new() {
        let arena: Arena<_, 1, 64> = Arena::new();
        let slot = ArenaArc::slot(&arena.insert(()));
        assert_eq!(ArenaArc::slot(&arena.remove(slot).unwrap()), slot);
    }

    #[test]
    fn test_with_capacity() {
        let arena: Arena<_, 1, 64> = Arena::with_capacity(0);
        let slot = ArenaArc::slot(&arena.insert(()));
        assert_eq!(ArenaArc::slot(&arena.remove(slot).unwrap()), slot);
    }

    /// Thread sanitizer produces false positive in this test.
    ///
    /// This has been discussed in
    /// [this issue](https://github.com/vorner/arc-swap/issues/71)
    /// and the failure can only be reproduced on x86-64-unknown-linux-gnu.
    /// It cannot be reproduced on MacOS.
    ///
    /// Since crate arc-swap is a cross platform crate with no assembly used
    /// or any x86 specific feature, this can be some bugs in the allocator
    /// or the thread sanitizer.
    #[cfg(not(feature = "thread-sanitizer"))]
    #[test]
    fn realworld_test() {
        use std::thread::sleep;
        use std::time::Duration;

        use parking_lot::Mutex;
        use rayon::prelude::*;
        use rayon::spawn;
        use std::sync::Arc;

        let arena: Arc<Arena<Mutex<u32>, 1, 64>> = Arc::new(Arena::with_capacity(0));

        (0..u16::MAX).into_par_iter().for_each(|i| {
            let i = i as u32;

            let arc = arena.insert(Mutex::new(i));

            assert_eq!(ArenaArc::strong_count(&arc), 2);
            assert_eq!(*arc.lock(), i);

            let slot = ArenaArc::slot(&arc);

            let arena = arena.clone();

            spawn(move || {
                sleep(Duration::from_micros(1));

                let arc = arena.remove(slot).unwrap();

                let mut guard = arc.lock();
                assert_eq!(*guard, i);
                *guard = 2000;
            });
        });
    }
}
