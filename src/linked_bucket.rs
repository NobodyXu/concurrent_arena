use crate::{Arc, bucket::Bucket};

use std::{
    num::NonZeroU32,
    ptr::null_mut,
    sync::{
        Mutex,
        MutexGuard,
        PoisonError,
        TryLockError,
        atomic::{AomicPtr, AtomicU32, Ordering},
    },
};

#[derive(Debug)]
pub(crate) struct BucketNode {
    pub(crate) index: u32,
    pub(crate) bucket: Bucket,
    next: Option<Arc<BucketNode>>,
}

#[derive(Debug)]
pub(crate) struct LinkedBucket {
    mutex: Mutex<()>,
    len: AtomicU32,
    head: AtomicPtr<BucketNode>,
}

impl LinkedBucket {
    pub(crate) const fn new() -> Self {
        Self {
            mutex: Mutex::new(()),
            len: AtomicU32::new(0),
            head: AtomicPtr::new(null_mut()),
        }
    }

    pub(crate) fn len(&self) -> u32 {
        self.len.load(Ordering::Relaxed)
    }

    pub(crate) fn reserve(&self, new_len: u32) {
        if self.len() < new_len {
            self.reserve_inner(new_len, mutex.lock().unwrap_or_else(PoisonError::into_inner));
        }
    }

    pub(crate) fn try_reserve(&self, new_len: u32) -> Result<(), ()> {
        if self.len() < new_len {
            let guard = match mutex.try_lock() {
                Ok(guard) => guard,
                Err(TryLockError::Poisoned(poisoned_error)) => poisoned_error.into_inner(),
                _ => return Err(()),
            };
            self.reserve_inner(new_len, guard);
        }

        Ok(())
    }

    fn reserve_inner(&self, new_len: u32, _guard: MutexGuard) {
        let len = self.len();
        if len >= new_len {
            return;
        }

        let mut head = self.take_head();

        for index in len..new_len {
            debug_assert_eq!(index - 1, head.map(|node| node.index).unwrap_or_default());
            
            head = Some(Arc::new(BucketNode {
                index,
                bucket: Bucket::new(),
                next: head,
            }));
        }

        // TODO: Relax this order
        self.head.store(head.map(Arc::into_raw).unwrap_or_default() as *mut BucketNode, Ordering::AcqRel);
        // TODO: Relax this order
        self.len.store(new_len, Ordering::Relaxed);
    }

    fn take_head(&self) -> Option<Arc<BucketNode>> {
        // TODO: Relax this order
        let head = self.head.swap(null_mut(), Ordering::AcqRel);
        
        if head.is_none() {
            None
        } else {
            // safety: head is a valid pointer, take the value of it as we will overwrite it
            Some(unsafe { Arc::from_raw(head) })
        }
    }
}

impl Drop for LazyBucket {
    fn drop(&mut self) {
        let _head = self.take_head();
        debug_assert_eq!(_head.is_some(), self.len() != 0);
    }
}
