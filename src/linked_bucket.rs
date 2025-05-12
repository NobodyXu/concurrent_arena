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
struct BucketNode {
    bucket: Bucket,
    next: Option<Arc<BucketNode>>,
}

#[derive(Debug)]
pub(crate) struct LinkedBucket {
    mutex: Mutex<()>,
    /// Atomic Arc
    head: AtomicPtr<BucketNode>,
}

impl LinkedBucket {
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
