use super::bitmap::BitMap;

use core::cell::UnsafeCell;
use core::ops::Deref;

use std::sync::atomic::{fence, AtomicU8, Ordering};
use std::sync::Arc;

use array_init::array_init;

const REMOVED_MASK: u8 = 1 << (u8::BITS - 1);
const REFCNT_MASK: u8 = !REMOVED_MASK;
pub const MAX_REFCNT: u8 = REFCNT_MASK;

#[derive(Debug)]
struct Entry<T> {
    counter: AtomicU8,
    val: UnsafeCell<Option<T>>,
}

impl<T> Default for Entry<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> Entry<T> {
    fn new() -> Self {
        Self {
            counter: AtomicU8::new(0),
            val: UnsafeCell::new(None),
        }
    }
}

#[derive(Debug)]
pub(crate) struct Bucket<T, const BITARRAY_LEN: usize, const LEN: usize> {
    bitset: BitMap<BITARRAY_LEN>,
    entries: [Entry<T>; LEN],
}

impl<T, const BITARRAY_LEN: usize, const LEN: usize> Bucket<T, BITARRAY_LEN, LEN> {
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

        // 1 for the ArenaArc, another is for the Bucket itself.
        //
        // Use `Acquire` here to make sure drop is written to memory before
        // the entry is reused again.
        let prev_refcnt = entry.counter.swap(2, Ordering::Acquire);
        debug_assert_eq!(prev_refcnt, 0);

        let option = unsafe { &mut *entry.val.get() };
        debug_assert!(option.is_none());
        *option = Some(value);

        let index = index as u32;

        Ok(ArenaArc {
            slot: bucket_index * (LEN as u32) + index,
            index,
            bucket: Arc::clone(this),
        })
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
pub struct ArenaArc<T, const BITARRAY_LEN: usize, const LEN: usize> {
    slot: u32,
    index: u32,
    bucket: Arc<Bucket<T, BITARRAY_LEN, LEN>>,
}

impl<T, const BITARRAY_LEN: usize, const LEN: usize> ArenaArc<T, BITARRAY_LEN, LEN> {
    pub fn slot(&self) -> u32 {
        self.slot
    }

    fn get_index(&self) -> usize {
        self.index as usize
    }

    fn get_entry(&self) -> &Entry<T> {
        let entry = &self.bucket.entries[self.get_index()];
        debug_assert!((entry.counter.load(Ordering::Relaxed) & REFCNT_MASK) > 0);
        entry
    }
}

impl<T, const BITARRAY_LEN: usize, const LEN: usize> Deref for ArenaArc<T, BITARRAY_LEN, LEN> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        let ptr = self.get_entry().val.get() as *const T;

        unsafe { &*ptr }
    }
}

impl<T, const BITARRAY_LEN: usize, const LEN: usize> Clone for ArenaArc<T, BITARRAY_LEN, LEN> {
    fn clone(&self) -> Self {
        let entry = self.get_entry();

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

impl<T, const BITARRAY_LEN: usize, const LEN: usize> Drop for ArenaArc<T, BITARRAY_LEN, LEN> {
    fn drop(&mut self) {
        let entry = self.get_entry();

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

            self.bucket.bitset.deallocate(self.get_index());
        }
    }
}
