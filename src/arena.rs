use super::bucket::Bucket;
use super::thread_id::get_thread_id;
use super::ArenaArc;

use core::cmp::min;

use std::sync::Arc;

use arrayvec::ArrayVec;

use parking_lot::const_rwlock;
use parking_lot::RwLock;
use parking_lot::RwLockUpgradableReadGuard;
use parking_lot::RwLockWriteGuard;

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
#[derive(Debug)]
pub struct Arena<T, const BITARRAY_LEN: usize, const LEN: usize> {
    buckets: RwLock<Vec<Arc<Bucket<T, BITARRAY_LEN, LEN>>>>,
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

    pub const fn const_new() -> Self {
        check_const_generics::<BITARRAY_LEN, LEN>();

        Self {
            buckets: const_rwlock(Vec::new()),
        }
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

        let mut buckets = Vec::with_capacity(cap as usize);
        for _ in 0..cap {
            buckets.push(Arc::new(Bucket::new()));
        }

        Self {
            buckets: RwLock::new(buckets),
        }
    }

    /// Return Ok(arc) on success, or Err((value, len)) where value is
    /// the input param `value` and `len` is the length of the `Arena` at the time
    /// of insertion.
    ///
    /// # How it works
    ///
    /// `try_insert` would acquire the read guard and iterate the entire underlying
    /// backing `Vec`, starting from a random position decided by the unique thread id.
    ///
    /// It would then try to insert a new entry in `Bucket`, which also iterate over
    /// the bitmap starting from a random position decided by the unique thread id.
    ///
    /// Using the approach described above to distribute insertion requests, we can
    /// minimize the possibility on two threads trying to access and modify the
    /// same variable using atomic instructions, thus improving efficiency.
    pub fn try_insert(&self, mut value: T) -> Result<ArenaArc<T, BITARRAY_LEN, LEN>, (T, u32)> {
        let guard = self.buckets.read();
        let len = guard.len();

        debug_assert!(len <= Self::max_buckets() as usize);

        if len == 0 {
            return Err((value, 0));
        }

        let mut pos = get_thread_id() % len;

        let slice1_iter = guard[pos..].iter();
        let slice2_iter = guard[..pos].iter();

        for bucket in slice1_iter.chain(slice2_iter) {
            match Bucket::try_insert(bucket, pos as u32, value) {
                Ok(arc) => return Ok(arc),
                Err(val) => value = val,
            }

            pos = (pos + 1) % len;
        }

        Err((value, len as u32))
    }

    /// * `BUFFER_SIZE` - Must be less than or equal to `Self::max_buckets()` and
    ///   greater than 0.
    ///
    ///   It will be used to store `Arc` (8 bytes on 64-bit platform and
    ///   4 bytes on 32-bit platform).
    ///
    ///   It is suggested to use any value between [10, 30].
    ///
    ///   And having `BUFFER_SIZE` larger than `new_len - len` would only waste stack.
    ///
    /// * `new_len` - For best performance, try to set this to number of CPUs
    ///   that are going to be concurrently access `Arena`.
    ///
    ///   If `new_len` is greater than `Self::max_buckets()`, then only
    ///   `Self::max_buckets()` will be reserved.
    ///
    /// Try to reserve `min(new_len, Self::max_buckets())` buckets.
    ///
    /// In order to reduce critical section, `reserve` would try to first acquire
    /// upgradable read guard, which would not block other readers, but
    /// do block other threads attempting to acquire upgradable read guard
    /// and write guard.
    ///
    /// If it fails to acquire the upgradable read lock, it would return `false`.
    ///
    /// This would reduce number of threads waiting to reserve more buckets,
    /// thus reducing the contention for upgradable read lock.
    ///
    /// These threads could give up reservation and try to `insert` new keys instead,
    /// as others might have already reserve enough space for it.
    ///
    /// It would check the length, and if it is not large enough, then it would
    /// allocate the buckets on the buffer, upgrade the read guard to
    /// write guard and move the buckets in the buffer into the underlying vec
    /// used by `Arena`.
    ///
    /// If `new_len <= len`, then return `true`.
    ///
    /// If the buffer isn't large enough for all elements, it will downgrade
    /// the write guard to upgradable read guard and do the steps described above
    /// again.
    ///
    /// After `new_len - len` new buckets are added, return `true`.
    pub fn try_reserve<const BUFFER_SIZE: usize>(&self, new_len: u32) -> bool {
        cfn_assert!(BUFFER_SIZE <= Self::max_buckets() as usize);
        cfn_assert!(BUFFER_SIZE > 0);

        if new_len == 0 {
            return true;
        }

        let new_len = min(new_len, Self::max_buckets());
        let mut buffer = ArrayVec::<Arc<Bucket<T, BITARRAY_LEN, LEN>>, BUFFER_SIZE>::new();

        // Use an upgradable_read to check if another thread has allocated
        // the buckets.
        //
        // Unlike write guard, this UpgradableReadGuard only blocks
        // other UpgradableReadGuard and WriteGuard, so the readers
        // will not be blocked while ensuring that there is no other
        // writer.
        let mut read_guard = match self.buckets.try_upgradable_read() {
            Some(read_guard) => read_guard,
            None => return false,
        };

        let len = read_guard.len() as u32;

        // If another writer has already done the reservation, return.
        if len >= new_len {
            return true;
        }

        let mut cnt = new_len - len;

        loop {
            // If no other writer has done the reservation, do it now.
            //
            // First, we allocate new bucket and put it in buffer
            // to avoid blocking the readers.
            for _ in 0..min(cnt, BUFFER_SIZE as u32) {
                buffer.push(Arc::new(Bucket::new()));
            }

            cnt -= buffer.len() as u32;

            // Push all allocated buckets into the buffer at once.
            let mut write_guard = RwLockUpgradableReadGuard::upgrade(read_guard);
            // Drain the buffer
            for new_bucket in buffer.drain(..) {
                write_guard.push(new_bucket);
            }

            if cnt == 0 {
                return true;
            }

            read_guard = RwLockWriteGuard::downgrade_to_upgradable(write_guard);
        }
    }

    /// Insert one value.
    ///
    /// If there isn't enough buckets, then try to reserve one bucket and
    /// restart the operation.
    pub fn insert(&self, mut value: T) -> ArenaArc<T, BITARRAY_LEN, LEN> {
        loop {
            match self.try_insert(value) {
                Ok(arc) => break arc,
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
                        self.try_reserve::<1>(len + 1);
                    }
                }
            }
        }
    }

    /// May enter busy loop if the slot is not fully initialized, however
    /// the busy loop does not hold the read lock of the underlying `Vec`.
    pub fn remove(&self, slot: u32) -> Option<ArenaArc<T, BITARRAY_LEN, LEN>> {
        let bucket_index = slot / (LEN as u32);
        let index = slot % (LEN as u32);

        let bucket = self.buckets.read()[bucket_index as usize].clone();

        Bucket::remove(bucket, bucket_index, index)
    }

    /// Return number of buckets allocated.
    pub fn len(&self) -> u32 {
        self.buckets.read().len() as u32
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod tests {
    use crate::*;

    use std::thread::sleep;
    use std::time::Duration;

    use parking_lot::Mutex;
    use rayon::prelude::*;
    use rayon::spawn;
    use std::sync::Arc;

    #[test]
    fn test_const_new() {
        let arena: Arena<_, 1, 64> = Arena::const_new();
        let slot = ArenaArc::slot(&arena.insert(()));
        assert_eq!(ArenaArc::slot(&arena.remove(slot).unwrap()), slot);
    }

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

    #[test]
    fn realworld_test() {
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
