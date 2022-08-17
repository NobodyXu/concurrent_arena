use super::{bitmap::BitMap, Arc, OptionExt};

use core::{cell::UnsafeCell, hint::spin_loop, ops::Deref};
use std::sync::atomic::{fence, AtomicU8, Ordering};

use array_init::array_init;

const REMOVED_MASK: u8 = 1 << (u8::BITS - 1);
const REFCNT_MASK: u8 = !REMOVED_MASK;
pub const MAX_REFCNT: u8 = REFCNT_MASK;

#[derive(Debug)]
struct Entry<T> {
    counter: AtomicU8,
    val: UnsafeCell<Option<T>>,
}

impl<T> Entry<T> {
    fn new() -> Self {
        Self {
            counter: AtomicU8::new(0),
            val: UnsafeCell::new(None),
        }
    }
}

impl<T> Drop for Entry<T> {
    fn drop(&mut self) {
        // Use `Acquire` here to make sure option is set to None before
        // the entry is dropped.
        let cnt = self.counter.load(Ordering::Acquire);

        // It must be either deleted, or is still alive
        // but no `ArenaArc` reference exist.
        debug_assert!(cnt <= 1);

        let val = self.val.get_mut().take();

        if cnt == 0 {
            debug_assert!(val.is_none());
        } else {
            debug_assert!(val.is_some());
        }
    }
}

#[derive(Debug)]
pub(crate) struct Bucket<T, const BITARRAY_LEN: usize, const LEN: usize> {
    bitset: BitMap<BITARRAY_LEN>,
    entries: [Entry<T>; LEN],
}

unsafe impl<T: Send + Sync, const BITARRAY_LEN: usize, const LEN: usize> Sync
    for Bucket<T, BITARRAY_LEN, LEN>
{
}

unsafe impl<T: Send + Sync, const BITARRAY_LEN: usize, const LEN: usize> Send
    for Bucket<T, BITARRAY_LEN, LEN>
{
}

impl<T: Send + Sync, const BITARRAY_LEN: usize, const LEN: usize> Default
    for Bucket<T, BITARRAY_LEN, LEN>
{
    fn default() -> Self {
        Self::new()
    }
}

impl<T: Send + Sync, const BITARRAY_LEN: usize, const LEN: usize> Bucket<T, BITARRAY_LEN, LEN> {
    pub(crate) fn new() -> Self {
        Self {
            bitset: BitMap::new(),
            entries: array_init(|_| Entry::new()),
        }
    }

    pub(crate) fn try_insert(
        this: &Arc<Self>,
        bucket_index: u32,
        value: T,
    ) -> Result<ArenaArc<T, BITARRAY_LEN, LEN>, T> {
        let index = match this.bitset.allocate() {
            Some(index) => index,
            None => return Err(value),
        };

        let entry = &this.entries[index];

        // Use `Acquire` here to make sure option is set to None before
        // the entry is reused again.
        let prev_refcnt = entry.counter.load(Ordering::Acquire);
        debug_assert_eq!(prev_refcnt, 0);

        let ptr = entry.val.get();
        let res = unsafe { ptr.replace(Some(value)) };
        debug_assert!(res.is_none());

        // 1 for the ArenaArc, another is for the Bucket itself.
        //
        // Set counter after option is set to `Some(...)` to avoid
        // race condition with `remove`.
        #[cfg(debug_assertions)]
        {
            let prev_refcnt = entry.counter.swap(2, Ordering::Relaxed);
            assert_eq!(prev_refcnt, 0);
        }
        #[cfg(not(debug_assertions))]
        {
            entry.counter.store(2, Ordering::Relaxed);
        }

        let index = index as u32;

        Ok(ArenaArc {
            slot: bucket_index * (LEN as u32) + index,
            index,
            bucket: Arc::clone(this),
        })
    }

    pub(crate) fn get(
        this: Arc<Self>,
        bucket_index: u32,
        index: u32,
    ) -> Option<ArenaArc<T, BITARRAY_LEN, LEN>> {
        if this.bitset.load(index) {
            let counter = &this.entries[index as usize].counter;
            let mut refcnt = counter.load(Ordering::Relaxed);

            loop {
                if (refcnt & REMOVED_MASK) != 0 {
                    return None;
                }

                if refcnt == 0 {
                    // The variable is not yet fully initialized.
                    // Reload the refcnt and check again.
                    spin_loop();
                    refcnt = counter.load(Ordering::Relaxed);
                    continue;
                }

                match counter.compare_exchange_weak(
                    refcnt,
                    refcnt + 1,
                    Ordering::Relaxed,
                    Ordering::Relaxed,
                ) {
                    Ok(_) => break,
                    Err(new_refcnt) => refcnt = new_refcnt,
                }
            }

            Some(ArenaArc {
                slot: bucket_index * (LEN as u32) + index,
                index,
                bucket: this,
            })
        } else {
            None
        }
    }

    pub(crate) fn remove(
        this: Arc<Self>,
        bucket_index: u32,
        index: u32,
    ) -> Option<ArenaArc<T, BITARRAY_LEN, LEN>> {
        if this.bitset.load(index) {
            let counter = &this.entries[index as usize].counter;
            let mut refcnt = counter.load(Ordering::Relaxed);

            loop {
                if (refcnt & REMOVED_MASK) != 0 {
                    return None;
                }

                if refcnt == 0 {
                    // The variable is not yet fully initialized.
                    // Reload the refcnt and check again.
                    spin_loop();
                    refcnt = counter.load(Ordering::Relaxed);
                    continue;
                }

                match counter.compare_exchange_weak(
                    refcnt,
                    refcnt | REMOVED_MASK,
                    Ordering::Relaxed,
                    Ordering::Relaxed,
                ) {
                    Ok(_) => break,
                    Err(new_refcnt) => refcnt = new_refcnt,
                }
            }

            Some(ArenaArc {
                slot: bucket_index * (LEN as u32) + index,
                index,
                bucket: this,
            })
        } else {
            None
        }
    }
}

/// Can have at most `MAX_REFCNT` refcount.
#[derive(Debug)]
pub struct ArenaArc<T: Send + Sync, const BITARRAY_LEN: usize, const LEN: usize> {
    slot: u32,
    index: u32,
    bucket: Arc<Bucket<T, BITARRAY_LEN, LEN>>,
}

impl<T: Send + Sync, const BITARRAY_LEN: usize, const LEN: usize> Unpin
    for ArenaArc<T, BITARRAY_LEN, LEN>
{
}

impl<T: Send + Sync, const BITARRAY_LEN: usize, const LEN: usize> ArenaArc<T, BITARRAY_LEN, LEN> {
    pub fn slot(this: &Self) -> u32 {
        this.slot
    }

    fn get_index(this: &Self) -> usize {
        this.index as usize
    }

    fn get_entry(this: &Self) -> &Entry<T> {
        let entry = &this.bucket.entries[Self::get_index(this)];
        debug_assert!((entry.counter.load(Ordering::Relaxed) & REFCNT_MASK) > 0);
        entry
    }

    pub fn strong_count(this: &Self) -> u8 {
        let entry = Self::get_entry(this);
        let cnt = entry.counter.load(Ordering::Relaxed) & REFCNT_MASK;
        debug_assert!(cnt > 0);
        cnt
    }

    pub fn is_removed(this: &Self) -> bool {
        let counter = &Self::get_entry(this).counter;
        let refcnt = counter.load(Ordering::Relaxed);

        (refcnt & REMOVED_MASK) != 0
    }

    /// Remove this element.
    ///
    /// Return true if succeeds, false if it is already removed.
    pub fn remove(this: &Self) -> bool {
        let counter = &Self::get_entry(this).counter;
        let mut refcnt = counter.load(Ordering::Relaxed);

        loop {
            debug_assert_ne!(refcnt & REFCNT_MASK, 0);

            if (refcnt & REMOVED_MASK) != 0 {
                // already removed
                return false;
            }

            // Since the element is not removed, there is at least two ref to it:
            //  - From the bucket itself
            //  - From `self`
            debug_assert_ne!(refcnt, 1);

            match counter.compare_exchange_weak(
                refcnt,
                // Reduce refcnt by one since it is removed from bucket.
                (refcnt - 1) | REMOVED_MASK,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => return true,
                Err(new_refcnt) => refcnt = new_refcnt,
            }
        }
    }
}

impl<T: Send + Sync, const BITARRAY_LEN: usize, const LEN: usize> Deref
    for ArenaArc<T, BITARRAY_LEN, LEN>
{
    type Target = T;

    fn deref(&self) -> &Self::Target {
        let ptr = Self::get_entry(self).val.get();

        unsafe { (*ptr).as_ref().unwrap_unchecked_on_release() }
    }
}

impl<T: Send + Sync, const BITARRAY_LEN: usize, const LEN: usize> Clone
    for ArenaArc<T, BITARRAY_LEN, LEN>
{
    fn clone(&self) -> Self {
        let entry = Self::get_entry(self);

        // According to [Boost documentation][1], increasing the refcount
        // can be done using Relaxed operation since there are at least one
        // reference alive.
        //
        // [1]: https://www.boost.org/doc/libs/1_77_0/doc/html/atomic/usage_examples.html
        if (entry.counter.fetch_add(1, Ordering::Relaxed) & REFCNT_MASK) == MAX_REFCNT {
            panic!("ArenaArc can have at most u8::MAX refcount");
        }

        Self {
            slot: self.slot,
            index: self.index,
            bucket: Arc::clone(&self.bucket),
        }
    }
}

impl<T: Send + Sync, const BITARRAY_LEN: usize, const LEN: usize> Drop
    for ArenaArc<T, BITARRAY_LEN, LEN>
{
    fn drop(&mut self) {
        let entry = Self::get_entry(self);

        // According to [Boost documentation][1], decreasing refcount must be done
        // using Release to ensure the write to the value happens before the
        // reference is dropped.
        //
        // [1]: https://www.boost.org/doc/libs/1_77_0/doc/html/atomic/usage_examples.html
        let prev_counter = entry.counter.fetch_sub(1, Ordering::Release);
        let prev_refcnt = prev_counter & MAX_REFCNT;

        debug_assert_ne!(prev_refcnt, 0);

        if prev_refcnt == 1 {
            debug_assert_eq!(prev_counter, REMOVED_MASK | 1);

            // This is the last reference, drop the value.

            // According to [Boost documentation][1], an Acquire fence must be used
            // before dropping value to ensure that all write to the value happens
            // before it is dropped.
            fence(Ordering::Acquire);

            // Now entry.counter == 0
            let option = unsafe { &mut *entry.val.get() };
            *option = None;

            // Make sure drop is written to memory before
            // the entry is reused again.
            entry.counter.store(0, Ordering::Release);

            self.bucket.bitset.deallocate(Self::get_index(self));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Arc;
    use super::ArenaArc;

    use parking_lot::Mutex;
    use parking_lot::MutexGuard;

    use std::thread::sleep;
    use std::thread::spawn;
    use std::time::Duration;

    use rayon::prelude::*;

    type Bucket<T> = super::Bucket<T, 1, 64>;

    #[test]
    fn test_basic() {
        let bucket: Arc<Bucket<u32>> = Arc::new(Bucket::new());

        let arcs: Vec<_> = (0..64)
            .into_par_iter()
            .map(|i| {
                let arc = Bucket::try_insert(&bucket, 0, i).unwrap();

                assert_eq!(ArenaArc::strong_count(&arc), 2);
                assert_eq!(*arc, i);

                arc
            })
            .collect();

        assert!(Bucket::try_insert(&bucket, 0, 0).is_err());

        for (i, each) in arcs.iter().enumerate() {
            assert_eq!((**each) as usize, i);
        }

        let arcs_get: Vec<_> = (&arcs)
            .into_par_iter()
            .enumerate()
            .map(|(i, orig_arc)| {
                let arc = Bucket::get(Arc::clone(&bucket), 0, orig_arc.index).unwrap();

                assert_eq!(ArenaArc::strong_count(&arc), 3);
                assert_eq!(*arc as usize, i);

                arc
            })
            .collect();

        for (i, each) in arcs_get.iter().enumerate() {
            assert_eq!((**each) as usize, i);
        }
    }

    #[test]
    fn test_clone() {
        let bucket: Arc<Bucket<u32>> = Arc::new(Bucket::new());

        let arcs: Vec<_> = (0..64)
            .into_par_iter()
            .map(|i| {
                let arc = Bucket::try_insert(&bucket, 0, i).unwrap();

                assert_eq!(ArenaArc::strong_count(&arc), 2);
                assert_eq!(*arc, i);

                arc
            })
            .collect();

        let arcs_cloned: Vec<_> = arcs
            .iter()
            .map(|arc| {
                let new_arc = arc.clone();
                assert_eq!(ArenaArc::strong_count(&new_arc), 3);
                assert_eq!(ArenaArc::strong_count(arc), 3);

                new_arc
            })
            .collect();

        drop(arcs);
        drop(bucket);

        // bucket are dropped, however as long as the arcs
        // are alive, these values are still kept alive.
        for (i, each) in arcs_cloned.iter().enumerate() {
            assert_eq!((**each) as usize, i);
        }
    }

    #[test]
    fn test_reuse() {
        let bucket: Arc<Bucket<u32>> = Arc::new(Bucket::new());

        let mut arcs: Vec<_> = (0..64)
            .into_par_iter()
            .map(|i| {
                let arc = Bucket::try_insert(&bucket, 0, i).unwrap();

                assert_eq!(ArenaArc::strong_count(&arc), 2);
                assert_eq!(*arc, i);

                arc
            })
            .collect();

        for arc in arcs.drain(arcs.len() / 2..) {
            assert_eq!(ArenaArc::strong_count(&arc), 2);
            let new_arc = Bucket::remove(bucket.clone(), 0, arc.index).unwrap();
            assert_eq!(ArenaArc::strong_count(&arc), 2);

            assert!(ArenaArc::is_removed(&new_arc));

            drop(new_arc);
            assert_eq!(ArenaArc::strong_count(&arc), 1);
        }

        let new_arcs: Vec<_> = (64..64 + 32)
            .into_par_iter()
            .map(|i| {
                let arc = Bucket::try_insert(&bucket, 0, i).unwrap();

                assert_eq!(ArenaArc::strong_count(&arc), 2);
                assert_eq!(*arc, i);

                arc
            })
            .collect();

        let handle1 = spawn(move || {
            arcs.into_par_iter().enumerate().for_each(|(i, each)| {
                assert_eq!((*each) as usize, i);
            });
        });

        let handle2 = spawn(move || {
            new_arcs
                .into_par_iter()
                .zip(64..64 + 32)
                .for_each(|(each, i)| {
                    assert_eq!((*each) as usize, i);
                });
        });

        handle1.join().unwrap();
        handle2.join().unwrap();
    }

    #[test]
    fn test_reuse2() {
        let bucket: Arc<Bucket<u32>> = Arc::new(Bucket::new());

        let mut arcs: Vec<_> = (0..64)
            .into_par_iter()
            .map(|i| {
                let arc = Bucket::try_insert(&bucket, 0, i).unwrap();

                assert_eq!(ArenaArc::strong_count(&arc), 2);
                assert_eq!(*arc, i);

                arc
            })
            .collect();

        for arc in arcs.drain(arcs.len() / 2..) {
            assert_eq!(ArenaArc::strong_count(&arc), 2);
            ArenaArc::remove(&arc);
            assert!(ArenaArc::is_removed(&arc));
            assert_eq!(ArenaArc::strong_count(&arc), 1);
        }

        let new_arcs: Vec<_> = (64..64 + 32)
            .into_par_iter()
            .map(|i| {
                let arc = Bucket::try_insert(&bucket, 0, i).unwrap();

                assert_eq!(ArenaArc::strong_count(&arc), 2);
                assert_eq!(*arc, i);

                arc
            })
            .collect();

        let handle1 = spawn(move || {
            arcs.into_par_iter().enumerate().for_each(|(i, each)| {
                assert_eq!((*each) as usize, i);
            });
        });

        let handle2 = spawn(move || {
            new_arcs
                .into_par_iter()
                .zip(64..64 + 32)
                .for_each(|(each, i)| {
                    assert_eq!((*each) as usize, i);
                });
        });

        handle1.join().unwrap();
        handle2.join().unwrap();
    }

    #[test]
    fn test_concurrent_remove() {
        let bucket: Arc<Bucket<u32>> = Arc::new(Bucket::new());

        let arcs: Vec<_> = (0..64)
            .into_par_iter()
            .map(|i| {
                let arc = Bucket::try_insert(&bucket, 0, i).unwrap();

                assert_eq!(ArenaArc::strong_count(&arc), 2);
                assert_eq!(*arc, i);

                arc
            })
            .collect();

        arcs.into_par_iter().for_each(|arc| {
            assert_eq!(ArenaArc::strong_count(&arc), 2);
            let new_arc = Bucket::remove(bucket.clone(), 0, arc.index).unwrap();
            assert!(ArenaArc::is_removed(&new_arc));
            assert_eq!(ArenaArc::strong_count(&arc), 2);

            drop(new_arc);
            assert_eq!(ArenaArc::strong_count(&arc), 1);
        });
    }

    #[test]
    fn test_concurrent_remove2() {
        let bucket: Arc<Bucket<u32>> = Arc::new(Bucket::new());

        let arcs: Vec<_> = (0..64)
            .into_par_iter()
            .map(|i| {
                let arc = Bucket::try_insert(&bucket, 0, i).unwrap();

                assert_eq!(ArenaArc::strong_count(&arc), 2);
                assert_eq!(*arc, i);

                arc
            })
            .collect();

        arcs.into_par_iter().for_each(|arc| {
            assert_eq!(ArenaArc::strong_count(&arc), 2);
            ArenaArc::remove(&arc);
            assert!(ArenaArc::is_removed(&arc));
            assert_eq!(ArenaArc::strong_count(&arc), 1);
        });
    }

    #[test]
    fn realworld_test() {
        let bucket: Arc<Bucket<Mutex<u32>>> = Arc::new(Bucket::new());

        (0..64).into_par_iter().for_each(|i| {
            let arc = Bucket::try_insert(&bucket, 0, Mutex::new(i)).unwrap();

            assert_eq!(ArenaArc::strong_count(&arc), 2);
            assert_eq!(*arc.lock(), i);

            let arc_cloned = arc.clone();

            let f = move |mut guard: MutexGuard<'_, u32>| {
                if *guard == i {
                    *guard = i + 1;
                } else if *guard == i + 1 {
                    *guard = i + 2;
                } else {
                    panic!("");
                }
            };

            let handle = spawn(move || {
                sleep(Duration::from_micros(1));

                f(arc_cloned.lock());
            });

            spawn(move || {
                sleep(Duration::from_micros(1));
                f(arc.lock());

                handle.join().unwrap();

                assert_eq!(*arc.lock(), i + 2);
            });
        });
    }
}
