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
