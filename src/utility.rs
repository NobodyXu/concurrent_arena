use std::slice::SliceIndex;

pub(crate) trait OptionExt<T> {
    unsafe fn unwrap_unchecked_on_release(self) -> T;
}

impl<T> OptionExt<T> for Option<T> {
    unsafe fn unwrap_unchecked_on_release(self) -> T {
        if cfg!(debug_assertions) {
            self.unwrap()
        } else {
            self.unwrap_unchecked()
        }
    }
}

pub(crate) trait SliceExt<T> {
    unsafe fn get_unchecked_on_release<I>(&self, index: I) -> &<I as SliceIndex<[T]>>::Output
    where
        I: SliceIndex<[T]>;
}

impl<T> SliceExt<T> for [T] {
    unsafe fn get_unchecked_on_release<I>(&self, index: I) -> &<I as SliceIndex<[T]>>::Output
    where
        I: SliceIndex<[T]>,
    {
        if cfg!(debug_assertions) {
            self.get(index).unwrap()
        } else {
            self.get_unchecked(index)
        }
    }
}
