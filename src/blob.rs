use std::io;
use std::marker;
use std::mem;
use std::slice;

use crate::util::Binding;
use crate::{raw, Error, Object, Oid};

/// A structure to represent a git [blob][1]
///
/// [1]: http://git-scm.com/book/en/Git-Internals-Git-Objects
pub struct Blob<'repo> {
    // Pointer must never be NULL, and must always be valid to read and write.
    raw: *mut raw::git_blob,
    _marker: marker::PhantomData<Object<'repo>>,
}

impl<'repo> Blob<'repo> {
    /// Get the id (SHA1) of a repository blob
    pub fn id(&self) -> Oid {
        // SAFETY: git_blob_id() is passed a valid pointer to a blob
        // object; the returned git_oid is valid to create an Oid from.
        unsafe { Binding::from_raw(raw::git_blob_id(&*self.raw)) }
    }

    /// Determine if the blob content is most certainly binary or not.
    pub fn is_binary(&self) -> bool {
        // SAFETY: git_blob_is_binary() is passed a valid pointer to a blob
        // object.
        unsafe { raw::git_blob_is_binary(&*self.raw) == 1 }
    }

    /// Get the content of this blob.
    pub fn content(&self) -> &[u8] {
        // SAFETY: git_blob_rawcontent() is passed a valid pointer to a blob
        // object, as is git_blob_rawsize(). The constructed slice has the
        // correct length, and the use of a non-mutable reference means that
        // the underlying data will not be changed while the slice still exists.
        unsafe {
            let data = raw::git_blob_rawcontent(&*self.raw) as *const u8;
            let len = raw::git_blob_rawsize(&*self.raw) as usize;
            slice::from_raw_parts(data, len)
        }
    }

    /// Get the size in bytes of the contents of this blob.
    pub fn size(&self) -> usize {
        // SAFETY: git_blob_rawsize() is passed a valid pointer to a blob
        // object.
        unsafe { raw::git_blob_rawsize(&*self.raw) as usize }
    }

    /// Casts this Blob to be usable as an `Object`
    pub fn as_object(&self) -> &Object<'repo> {
        // SAFETY: Blob and Object are compatible, see into_object()
        // safety comment below.
        unsafe { &*(self as *const _ as *const Object<'repo>) }
    }

    /// Consumes Blob to be returned as an `Object`
    pub fn into_object(self) -> Object<'repo> {
        assert_eq!(mem::size_of_val(&self), mem::size_of::<Object<'_>>());
        // SAFETY: both Blob and Object have a single pointer field (git_blob
        // and git_object respectively), and a PhantomData marker with the
        // lifetime of the repository. The markers are trivially compatible.
        // The pointed-to types are compatible - the git_blob starts with a
        // git_object.
        unsafe { mem::transmute(self) }
    }
}

impl<'repo> Binding for Blob<'repo> {
    type Raw = *mut raw::git_blob;

    unsafe fn from_raw(raw: *mut raw::git_blob) -> Blob<'repo> {
        Blob {
            raw,
            _marker: marker::PhantomData,
        }
    }
    fn raw(&self) -> *mut raw::git_blob {
        self.raw
    }
}

impl<'repo> std::fmt::Debug for Blob<'repo> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> Result<(), std::fmt::Error> {
        f.debug_struct("Blob").field("id", &self.id()).finish()
    }
}

impl<'repo> Clone for Blob<'repo> {
    fn clone(&self) -> Self {
        self.as_object().clone().into_blob().ok().unwrap()
    }
}

impl<'repo> Drop for Blob<'repo> {
    fn drop(&mut self) {
        // SAFETY: git_blob_free() is passed a valid pointer to a blob object;
        // after this drop function it will never be accessed again.
        unsafe { raw::git_blob_free(self.raw) }
    }
}

/// A structure to represent a git writestream for blobs
pub struct BlobWriter<'repo> {
    // Pointer must never be NULL, and must always be valid to read and write.
    raw: *mut raw::git_writestream,
    need_cleanup: bool,
    _marker: marker::PhantomData<Object<'repo>>,
}

impl<'repo> BlobWriter<'repo> {
    /// Finalize blob writing stream and write the blob to the object db
    pub fn commit(mut self) -> Result<Oid, Error> {
        // After commit we already doesn't need cleanup on drop
        self.need_cleanup = false;
        let mut raw = crate::util::zeroed_raw_oid();
        // SAFETY: git_blob_create_fromstream_commit() is passed a valid pointer
        // to a write stream; the oid pointer provided is safe to write to.
        // Binding::from_raw() requires that it be provided a valid raw value;
        // try_call! will return before reaching that call if the
        // git_blob_create_fromstream_commit() call fails, if
        // Binding::from_raw() is reached then libgit2 will have set the `raw`
        // pointer to be valid.
        unsafe {
            try_call!(raw::git_blob_create_fromstream_commit(&mut raw, self.raw));
            Ok(Binding::from_raw(&raw as *const _))
        }
    }
}

impl<'repo> Binding for BlobWriter<'repo> {
    type Raw = *mut raw::git_writestream;

    unsafe fn from_raw(raw: *mut raw::git_writestream) -> BlobWriter<'repo> {
        BlobWriter {
            raw,
            need_cleanup: true,
            _marker: marker::PhantomData,
        }
    }
    fn raw(&self) -> *mut raw::git_writestream {
        self.raw
    }
}

impl<'repo> Drop for BlobWriter<'repo> {
    fn drop(&mut self) {
        // We need cleanup in case the stream has not been committed
        if self.need_cleanup {
            // SAFETY: if self.need_cleanup is false then the raw pointer is
            // valid to read from.
            if let Some(f) = unsafe { (*self.raw).free } {
                f(self.raw)
            }
        }
    }
}

impl<'repo> io::Write for BlobWriter<'repo> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        // SAFETY: if self.raw is valid to read from during the lifetime of the
        // BlobWriter.
        let write_cb = unsafe { (*self.raw).write };
        if let Some(f) = write_cb {
            let res = f(self.raw, buf.as_ptr() as *const _, buf.len());
            if res < 0 {
                Err(io::Error::other("Write error"))
            } else {
                Ok(buf.len())
            }
        } else {
            Err(io::Error::other("no write callback"))
        }
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::Repository;
    use std::fs::File;
    use std::io::prelude::*;
    use std::path::Path;
    use tempfile::TempDir;

    #[test]
    fn buffer() {
        let td = TempDir::new().unwrap();
        let repo = Repository::init(td.path()).unwrap();
        let id = repo.blob(&[5, 4, 6]).unwrap();
        let blob = repo.find_blob(id).unwrap();

        assert_eq!(blob.id(), id);
        assert_eq!(blob.size(), 3);
        assert_eq!(blob.content(), [5, 4, 6]);
        assert!(blob.is_binary());

        repo.find_object(id, None).unwrap().as_blob().unwrap();
        repo.find_object(id, None)
            .unwrap()
            .into_blob()
            .ok()
            .unwrap();
    }

    #[test]
    fn path() {
        let td = TempDir::new().unwrap();
        let path = td.path().join("foo");
        File::create(&path).unwrap().write_all(&[7, 8, 9]).unwrap();
        let repo = Repository::init(td.path()).unwrap();
        let id = repo.blob_path(&path).unwrap();
        let blob = repo.find_blob(id).unwrap();
        assert_eq!(blob.content(), [7, 8, 9]);
        blob.into_object();
    }

    #[test]
    fn stream() {
        let td = TempDir::new().unwrap();
        let repo = Repository::init(td.path()).unwrap();
        let mut ws = repo.blob_writer(Some(Path::new("foo"))).unwrap();
        let wl = ws.write(&[10, 11, 12]).unwrap();
        assert_eq!(wl, 3);
        let id = ws.commit().unwrap();
        let blob = repo.find_blob(id).unwrap();
        assert_eq!(blob.content(), [10, 11, 12]);
        blob.into_object();
    }
}
