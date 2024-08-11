struct SliceParent<'parent> {
    
}

/// A struct which allows reconstruction of slices.
struct Slice<'parent, T> {
    ptr: *const T,
    len: usize,
    parent: PhantomData<&'parent ()>
}

impl<'parent, T> Slice<'parent, T> {
    fn from_parent(slice: &'parent [T]) -> Self {

    }

    fn from_slice(slice: &[T]) -> Self {
        Self {
            ptr: slice.as_ptr(),
            len: slice.len()
        }
    }

    fn to_slice(slice: &[T]) -> Self {
        assert!()
    }
}