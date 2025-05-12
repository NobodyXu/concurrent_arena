use crate::{Arc, bucket::Bucket};

use std::{
    num::NonZeroU32,
    ptr::null_mut,
    sync::{
        Mutex,
        atomic::{AomicPtr, Ordering},
    },
};

#[derive(Debug)]
struct LinkedBucket {
    bucket: Bucket,
    next: Option<Arc<LinkedBucket>>,
}

#[derive(Debug)]
pub(crate) struct LazyBucket {
    mutex: Mutex<()>,
    /// Atomic Arc
    head: AtomicPtr<LinkedBucket>,
}

impl LazyBucket {
    pub const fn new() -> Self {
        Self {
            mutex: Mutex::new(()),
            head: AtomicPtr::new(null_mut()),
        }
    }

    pub fn allocate_buckets(&self, count: NonZeroU32) {}
}

impl Drop for LazyBucket {
    fn drop(&mut self) {}
}
