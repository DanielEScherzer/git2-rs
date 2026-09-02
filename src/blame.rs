use crate::util::{self, Binding};
use crate::{raw, signature, Error, ErrorClass, ErrorCode, Oid, Repository, Signature};
use libc::c_char;
use std::iter::FusedIterator;
use std::mem;
use std::ops::Range;
use std::path::Path;
use std::{marker, ptr};

/// Opaque structure to hold blame results.
pub struct Blame<'repo> {
    // Pointer must never be NULL, and must always be valid to read and write.
    raw: *mut raw::git_blame,
    _marker: marker::PhantomData<&'repo Repository>,
}

/// Structure that represents a blame hunk.
pub struct BlameHunk<'blame> {
    // Pointer must never be NULL, and must always be valid to read and write.
    raw: *mut raw::git_blame_hunk,
    _marker: marker::PhantomData<&'blame raw::git_blame>,
}

/// Blame options
pub struct BlameOptions {
    raw: raw::git_blame_options,
}

/// An iterator over the hunks in a blame.
pub struct BlameIter<'blame> {
    range: Range<usize>,
    blame: &'blame Blame<'blame>,
}

impl<'repo> Blame<'repo> {
    /// Get blame data for a file that has been modified in memory.
    ///
    /// Lines that differ between the buffer and the committed version are
    /// marked as having a zero OID for their final_commit_id.
    pub fn blame_buffer(&self, buffer: &[u8]) -> Result<Blame<'_>, Error> {
        // If the buffer is empty, and libgit2 has assertions enabled, it will
        // abort due to a failing assertion. If libgit2 is in release mode and
        // does not have assertions enabled it will instead emit an error;
        // recreate that error handling here (but with a better message) to
        // avoid aborting.
        if buffer.is_empty() {
            return Err(Error::new(
                // Matches libgit2
                ErrorCode::GenericError,
                // Matches libgit2
                ErrorClass::Invalid,
                // libgit2 would say "invalid argument: 'buffer && buffer_len'"
                // but let's have a nicer message
                "buffer cannot be empty",
            ));
        }
        let mut raw = ptr::null_mut();

        // SAFETY:
        // - git_blame_buffer()'s first parameter is writable double pointer;
        //   the mutable reference to the pointer created above is valid.
        // - git_blame_buffer()'s second parameter is a pointer to a valid
        //   git_blame; self.raw is such a pointer.
        // - git_blame_buffer()'s third parameter is a pointer to the file
        //   contents, which is what we provide; there is no restriction on
        //   interior null bytes given the fourth parameter.
        // - git_blame_buffer()'s fourth parameter is the length of the file
        //   contents that can be read; that length is provided.
        // - Binding::from_raw() requires that it be provided a valid raw
        //   value; try_call! will return before reaching that call if the
        //   git_blame_buffer() call fails, if Binding::from_raw() is reached
        //   then libgit2 will have set the `raw` pointer to be valid.
        unsafe {
            try_call!(raw::git_blame_buffer(
                &mut raw,
                self.raw,
                buffer.as_ptr() as *const c_char,
                buffer.len()
            ));
            Ok(Binding::from_raw(raw))
        }
    }

    /// Gets the number of hunks that exist in the blame structure.
    pub fn len(&self) -> usize {
        // SAFETY: git_blame_get_hunk_count() is passed a valid pointer to a
        // blame object.
        unsafe { raw::git_blame_get_hunk_count(self.raw) as usize }
    }

    /// Return `true` is there is no hunk in the blame structure.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Gets the blame hunk at the given index.
    pub fn get_index(&self, index: usize) -> Option<BlameHunk<'_>> {
        // SAFETY: git_blame_get_hunk_byindex() is passed a valid pointer to a
        // blame object.
        let ptr = unsafe { raw::git_blame_get_hunk_byindex(self.raw(), index as u32) };
        if ptr.is_null() {
            None
        } else {
            // SAFETY: if git_blame_get_hunk_byindex() returns non-NULL then it
            // returns a valid git_blame_hunk pointer.
            Some(unsafe { BlameHunk::from_raw_const(ptr) })
        }
    }

    /// Gets the hunk that relates to the given line number in the newest
    /// commit.
    pub fn get_line(&self, lineno: usize) -> Option<BlameHunk<'_>> {
        // SAFETY: git_blame_get_hunk_byline() is passed a valid pointer to a
        // blame object.
        let ptr = unsafe { raw::git_blame_get_hunk_byline(self.raw(), lineno) };
        if ptr.is_null() {
            None
        } else {
            // SAFETY: if git_blame_get_hunk_byline() returns non-NULL then it
            // returns a valid git_blame_hunk pointer.
            Some(unsafe { BlameHunk::from_raw_const(ptr) })
        }
    }

    /// Returns an iterator over the hunks in this blame.
    pub fn iter(&self) -> BlameIter<'_> {
        BlameIter {
            range: 0..self.len(),
            blame: self,
        }
    }
}

impl<'blame> BlameHunk<'blame> {
    unsafe fn from_raw_const(raw: *const raw::git_blame_hunk) -> BlameHunk<'blame> {
        BlameHunk {
            raw: raw as *mut raw::git_blame_hunk,
            _marker: marker::PhantomData,
        }
    }

    /// Returns OID of the commit where this line was last changed
    pub fn final_commit_id(&self) -> Oid {
        // SAFETY: per the BlameHunk invariants the raw pointer is always safe
        // to read; Oid::from_raw() is called with a valid git_oid_t since the
        // git_blame_hunk stores the actual git_oid_t, not a pointer to it.
        unsafe { Oid::from_raw(&(*self.raw).final_commit_id) }
    }

    /// Returns signature for the author of the final commit, if present.
    ///
    /// The final commit is the one identified by [Self::final_commit_id()].
    pub fn final_signature(&self) -> Option<Signature<'_>> {
        // SAFETY: per the BlameHunk invariants the raw pointer is always safe
        // to read.
        let ptr = unsafe { (*self.raw).final_signature };
        if ptr.is_null() {
            None
        } else {
            // SAFETY: if present, the pointer will be a valid signature
            // pointer; that pointer will be valid at least as long as the
            // current hunk is around.
            Some(unsafe { signature::from_raw_const(self, ptr) })
        }
    }

    /// Returns signature for the committer of the final commit, if present.
    ///
    /// The final commit is the one identified by [Self::final_commit_id()].
    pub fn final_committer(&self) -> Option<Signature<'_>> {
        // SAFETY: per the BlameHunk invariants the raw pointer is always safe
        // to read.
        let ptr = unsafe { (*self.raw).final_committer };
        if ptr.is_null() {
            None
        } else {
            // SAFETY: if present, the pointer will be a valid signature
            // pointer; that pointer will be valid at least as long as the
            // current hunk is around.
            Some(unsafe { signature::from_raw_const(self, ptr) })
        }
    }

    /// Returns line number where this hunk begins.
    ///
    /// Note that the start line is counting from 1.
    pub fn final_start_line(&self) -> usize {
        // SAFETY: per the BlameHunk invariants the raw pointer is always safe
        // to read.
        unsafe { (*self.raw).final_start_line_number }
    }

    /// Returns the OID of the commit where this hunk was found.
    ///
    /// This will usually be the same as `final_commit_id`,
    /// except when `BlameOptions::track_copies_any_commit_copies` has been
    /// turned on
    pub fn orig_commit_id(&self) -> Oid {
        // SAFETY: per the BlameHunk invariants the raw pointer is always safe
        // to read; Oid::from_raw() is called with a valid git_oid_t since the
        // git_blame_hunk stores the actual git_oid_t, not a pointer to it.
        unsafe { Oid::from_raw(&(*self.raw).orig_commit_id) }
    }

    /// Returns signature of the author of the original commit, if present.
    ///
    /// The original commit is the one identified by [Self::orig_commit_id()].
    pub fn orig_signature(&self) -> Option<Signature<'_>> {
        // SAFETY: per the BlameHunk invariants the raw pointer is always safe
        // to read.
        let ptr = unsafe { (*self.raw).orig_signature };
        if ptr.is_null() {
            None
        } else {
            // SAFETY: if present, the pointer will be a valid signature
            // pointer; that pointer will be valid at least as long as the
            // current hunk is around.
            Some(unsafe { signature::from_raw_const(self, ptr) })
        }
    }

    /// Returns signature of the committer of the original commit, if present.
    ///
    /// The original commit is the one identified by [Self::orig_commit_id()].
    pub fn orig_committer(&self) -> Option<Signature<'_>> {
        // SAFETY: per the BlameHunk invariants the raw pointer is always safe
        // to read.
        let ptr = unsafe { (*self.raw).orig_committer };
        if ptr.is_null() {
            None
        } else {
            // SAFETY: if present, the pointer will be a valid signature
            // pointer; that pointer will be valid at least as long as the
            // current hunk is around.
            Some(unsafe { signature::from_raw_const(self, ptr) })
        }
    }

    /// Returns line number where this hunk begins.
    ///
    /// Note that the start line is counting from 1.
    pub fn orig_start_line(&self) -> usize {
        // SAFETY: per the BlameHunk invariants the raw pointer is always safe
        // to read.
        unsafe { (*self.raw).orig_start_line_number }
    }

    /// Returns path to the file where this hunk originated.
    ///
    /// Note: `None` could be returned for non-unicode paths on Windows.
    pub fn path(&self) -> Option<&Path> {
        // SAFETY: per the BlameHunk invariants the raw pointer is always safe
        // to read; if the path is pesent, it is a valid c-string pointer and
        // will not be modified, meaning it can be used for reads with a
        // reference.
        let opt_bytes = unsafe { crate::opt_bytes(self, (*self.raw).orig_path) };
        if let Some(bytes) = opt_bytes {
            Some(util::bytes2path(bytes))
        } else {
            None
        }
    }

    /// Tests whether this hunk has been tracked to a boundary commit
    /// (the root, or the commit specified in git_blame_options.oldest_commit).
    pub fn is_boundary(&self) -> bool {
        // SAFETY: per the BlameHunk invariants the raw pointer is always safe
        // to read.
        unsafe { (*self.raw).boundary == 1 }
    }

    /// Returns number of lines in this hunk.
    pub fn lines_in_hunk(&self) -> usize {
        // SAFETY: per the BlameHunk invariants the raw pointer is always safe
        // to read.
        unsafe { (*self.raw).lines_in_hunk as usize }
    }

    /// Get the short "summary" of the git commit message for the hunk.
    ///
    /// The returned message is the summary of the commit, comprising the first
    /// paragraph of the message with whitespace trimmed and squashed.
    ///
    /// `Ok(None)` may be returned if there is no summary.
    pub fn summary(&self) -> Result<Option<&str>, Error> {
        match self.summary_bytes() {
            Some(sb) => str::from_utf8(sb).map(Some).map_err(|e| e.into()),
            None => Ok(None),
        }
    }

    /// Get the short "summary" of the git commit message for the hunk.
    ///
    /// The returned message is the summary of the commit, comprising the first
    /// paragraph of the message with whitespace trimmed and squashed.
    ///
    /// `None` may be returned if an error occurs
    pub fn summary_bytes(&self) -> Option<&[u8]> {
        // SAFETY: per the BlameHunk invariants the raw pointer is always safe
        // to read; if the summary is pesent, it is a valid c-string pointer and
        // will not be modified, meaning it can be used for reads with a
        // reference.
        unsafe { crate::opt_bytes(self, (*self.raw).summary) }
    }
}

impl Default for BlameOptions {
    fn default() -> Self {
        Self::new()
    }
}

impl BlameOptions {
    /// Initialize options
    pub fn new() -> BlameOptions {
        // SAFETY: the zero-initialized blame options may not be fully valid
        // but they are only used if the propert initialization by libgit2 below
        // is successful.
        let mut raw: raw::git_blame_options = unsafe { mem::zeroed() };

        assert_eq!(
            // SAFETY: the pointer provided comes from a mutable reference;
            // since we have a reference, the pointer is valid, and since it
            // is a mutable reference, it is valid to write to.
            unsafe { raw::git_blame_init_options(&mut raw, raw::GIT_BLAME_OPTIONS_VERSION) },
            0
        );

        // SAFETY: the raw git_blame_options object is valid and can be
        // converted into a pointer.
        unsafe { Binding::from_raw(&raw as *const _ as *mut _) }
    }

    fn flag(&mut self, opt: u32, val: bool) -> &mut BlameOptions {
        if val {
            self.raw.flags |= opt;
        } else {
            self.raw.flags &= !opt;
        }
        self
    }

    /// Track lines that have moved within a file.
    pub fn track_copies_same_file(&mut self, opt: bool) -> &mut BlameOptions {
        self.flag(raw::GIT_BLAME_TRACK_COPIES_SAME_FILE, opt)
    }

    /// Track lines that have moved across files in the same commit.
    pub fn track_copies_same_commit_moves(&mut self, opt: bool) -> &mut BlameOptions {
        self.flag(raw::GIT_BLAME_TRACK_COPIES_SAME_COMMIT_MOVES, opt)
    }

    /// Track lines that have been copied from another file that exists
    /// in the same commit.
    pub fn track_copies_same_commit_copies(&mut self, opt: bool) -> &mut BlameOptions {
        self.flag(raw::GIT_BLAME_TRACK_COPIES_SAME_COMMIT_COPIES, opt)
    }

    /// Track lines that have been copied from another file that exists
    /// in any commit.
    pub fn track_copies_any_commit_copies(&mut self, opt: bool) -> &mut BlameOptions {
        self.flag(raw::GIT_BLAME_TRACK_COPIES_ANY_COMMIT_COPIES, opt)
    }

    /// Restrict the search of commits to those reachable following only
    /// the first parents.
    pub fn first_parent(&mut self, opt: bool) -> &mut BlameOptions {
        self.flag(raw::GIT_BLAME_FIRST_PARENT, opt)
    }

    /// Use mailmap file to map author and committer names and email addresses
    /// to canonical real names and email addresses. The mailmap will be read
    /// from the working directory, or HEAD in a bare repository.
    pub fn use_mailmap(&mut self, opt: bool) -> &mut BlameOptions {
        self.flag(raw::GIT_BLAME_USE_MAILMAP, opt)
    }

    /// Ignore whitespace differences.
    pub fn ignore_whitespace(&mut self, opt: bool) -> &mut BlameOptions {
        self.flag(raw::GIT_BLAME_IGNORE_WHITESPACE, opt)
    }

    /// Setter for the id of the newest commit to consider.
    pub fn newest_commit(&mut self, id: Oid) -> &mut BlameOptions {
        // SAFETY: Oid::raw() returns a raw pointer that is safe to dereference
        // into a git_oid object.
        unsafe {
            self.raw.newest_commit = *id.raw();
        }
        self
    }

    /// Setter for the id of the oldest commit to consider.
    pub fn oldest_commit(&mut self, id: Oid) -> &mut BlameOptions {
        // SAFETY: Oid::raw() returns a raw pointer that is safe to dereference
        // into a git_oid object.
        unsafe {
            self.raw.oldest_commit = *id.raw();
        }
        self
    }

    /// The first line in the file to blame.
    pub fn min_line(&mut self, lineno: usize) -> &mut BlameOptions {
        self.raw.min_line = lineno;
        self
    }

    /// The last line in the file to blame.
    pub fn max_line(&mut self, lineno: usize) -> &mut BlameOptions {
        self.raw.max_line = lineno;
        self
    }
}

impl<'repo> Binding for Blame<'repo> {
    type Raw = *mut raw::git_blame;

    unsafe fn from_raw(raw: *mut raw::git_blame) -> Blame<'repo> {
        Blame {
            raw,
            _marker: marker::PhantomData,
        }
    }

    fn raw(&self) -> *mut raw::git_blame {
        self.raw
    }
}

#[expect(clippy::undocumented_unsafe_blocks)]
impl<'repo> Drop for Blame<'repo> {
    fn drop(&mut self) {
        unsafe { raw::git_blame_free(self.raw) }
    }
}

impl<'blame> Binding for BlameHunk<'blame> {
    type Raw = *mut raw::git_blame_hunk;

    unsafe fn from_raw(raw: *mut raw::git_blame_hunk) -> BlameHunk<'blame> {
        BlameHunk {
            raw,
            _marker: marker::PhantomData,
        }
    }

    fn raw(&self) -> *mut raw::git_blame_hunk {
        self.raw
    }
}

impl Binding for BlameOptions {
    type Raw = *mut raw::git_blame_options;

    unsafe fn from_raw(opts: *mut raw::git_blame_options) -> BlameOptions {
        BlameOptions { raw: *opts }
    }

    fn raw(&self) -> *mut raw::git_blame_options {
        &self.raw as *const _ as *mut _
    }
}

impl<'blame> Iterator for BlameIter<'blame> {
    type Item = BlameHunk<'blame>;
    fn next(&mut self) -> Option<BlameHunk<'blame>> {
        self.range.next().and_then(|i| self.blame.get_index(i))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.range.size_hint()
    }
}

impl<'blame> DoubleEndedIterator for BlameIter<'blame> {
    fn next_back(&mut self) -> Option<BlameHunk<'blame>> {
        self.range.next_back().and_then(|i| self.blame.get_index(i))
    }
}

impl<'blame> FusedIterator for BlameIter<'blame> {}

impl<'blame> ExactSizeIterator for BlameIter<'blame> {}

#[cfg(test)]
mod tests {
    use std::fs::{self, File};
    use std::path::Path;

    #[test]
    fn smoke() {
        let (_td, repo) = crate::test::repo_init();
        let mut index = repo.index().unwrap();

        let root = repo.workdir().unwrap();
        fs::create_dir(root.join("foo")).unwrap();
        File::create(root.join("foo/bar")).unwrap();
        index.add_path(Path::new("foo/bar")).unwrap();

        let committer_sig = crate::Signature::now("FizzBuzz", "bar@example.com")
            .expect("Signature creation should succeed");

        let id = index.write_tree().unwrap();
        let tree = repo.find_tree(id).unwrap();
        let sig = repo.signature().unwrap();
        let id = repo.refname_to_id("HEAD").unwrap();
        let parent = repo.find_commit(id).unwrap();
        let commit = repo
            .commit(
                Some("HEAD"),
                &sig,
                &committer_sig,
                "commit",
                &tree,
                &[&parent],
            )
            .unwrap();

        let blame = repo.blame_file(Path::new("foo/bar"), None).unwrap();

        assert_eq!(blame.len(), 1);
        assert_eq!(blame.iter().count(), 1);

        let hunk = blame.get_index(0).unwrap();
        assert_eq!(hunk.final_commit_id(), commit);
        assert_eq!(hunk.final_signature().unwrap().name(), sig.name());
        assert_eq!(hunk.final_signature().unwrap().email(), sig.email());
        assert_eq!(hunk.orig_signature().unwrap().name(), sig.name());
        assert_eq!(hunk.orig_signature().unwrap().email(), sig.email());
        assert_eq!(hunk.final_committer().unwrap().name(), committer_sig.name());
        assert_eq!(
            hunk.final_committer().unwrap().email(),
            committer_sig.email()
        );
        assert_eq!(hunk.orig_committer().unwrap().name(), committer_sig.name());
        assert_eq!(
            hunk.orig_committer().unwrap().email(),
            committer_sig.email()
        );
        assert_eq!(hunk.final_start_line(), 1);
        assert_eq!(hunk.path(), Some(Path::new("foo/bar")));
        assert_eq!(hunk.lines_in_hunk(), 0);
        assert_eq!(hunk.summary(), Ok(Some("commit")));
        assert!(!hunk.is_boundary());

        let blame_buffer = blame.blame_buffer("\n".as_bytes()).unwrap();
        let line = blame_buffer.get_line(1).unwrap();

        assert_eq!(blame_buffer.len(), 2);
        assert_eq!(blame_buffer.iter().count(), 2);
        assert!(line.final_commit_id().is_zero());
    }

    #[test]
    fn buffer_signatures() {
        // Regression tests for #1253
        let td = tempfile::TempDir::new().unwrap();
        let path = td.path();

        let repo = crate::Repository::init(path).unwrap();

        {
            let mut config = repo.config().unwrap();
            config.set_str("user.name", "name").unwrap();
            config.set_str("user.email", "email").unwrap();

            fs::write(path.join("README.md"), "Testing").unwrap();

            let mut index = repo.index().unwrap();
            index.add_path(Path::new("README.md")).unwrap();
            index.write().unwrap();

            let id = index.write_tree().unwrap();
            let tree = repo.find_tree(id).unwrap();
            let sig = repo.signature().unwrap();
            repo.commit(Some("HEAD"), &sig, &sig, "Add README.md", &tree, &[])
                .unwrap();
        }

        let blame = repo.blame_file(Path::new("README.md"), None).unwrap();
        // This hunk is safe to use
        let hunk = blame.get_index(0).unwrap();

        {
            let final_author = hunk.final_signature().unwrap();
            assert_eq!(Ok("name"), final_author.name());
            assert_eq!(Ok("email"), final_author.email());

            let final_committer = hunk.final_committer().unwrap();
            assert_eq!(Ok("name"), final_committer.name());
            assert_eq!(Ok("email"), final_committer.email());

            let original_author = hunk.orig_signature().unwrap();
            assert_eq!(Ok("name"), original_author.name());
            assert_eq!(Ok("email"), original_author.email());

            let original_committer = hunk.orig_committer().unwrap();
            assert_eq!(Ok("name"), original_committer.name());
            assert_eq!(Ok("email"), original_committer.email());
        }

        let arbitrary = blame.blame_buffer(b"abc123").unwrap();
        let hunk = arbitrary.get_index(0).unwrap();
        // This hunk is NOT safe to use
        // the final_signature, final_committer, orig_signature, and
        // orig_committer pointers are all NULL
        // But the other methods still work
        {
            let final_commit_id = hunk.final_commit_id();
            assert!(final_commit_id.is_zero());

            let original_commit_id = hunk.orig_commit_id();
            assert!(original_commit_id.is_zero());

            assert_eq!(1, hunk.final_start_line());
            assert_eq!(0, hunk.orig_start_line());
            assert_eq!(Some(Path::new("README.md")), hunk.path());
            assert!(!hunk.is_boundary());
            assert_eq!(1, hunk.lines_in_hunk());
            assert_eq!(Ok(None), hunk.summary());
            assert_eq!(None, hunk.summary_bytes());
        }

        {
            let final_author = hunk.final_signature();
            assert!(final_author.is_none());

            let final_committer = hunk.final_committer();
            assert!(final_committer.is_none());

            let original_author = hunk.orig_signature();
            assert!(original_author.is_none());

            let original_committer = hunk.orig_committer();
            assert!(original_committer.is_none());
        }
    }

    #[test]
    fn buffer_empty() {
        // Regression tests for #1288
        let td = tempfile::TempDir::new().unwrap();
        let path = td.path();

        let repo = crate::Repository::init(path).unwrap();

        {
            let mut config = repo.config().unwrap();
            config.set_str("user.name", "name").unwrap();
            config.set_str("user.email", "email").unwrap();

            fs::write(path.join("README.md"), "Testing").unwrap();

            let mut index = repo.index().unwrap();
            index.add_path(Path::new("README.md")).unwrap();
            index.write().unwrap();

            let id = index.write_tree().unwrap();
            let tree = repo.find_tree(id).unwrap();
            let sig = repo.signature().unwrap();
            repo.commit(Some("HEAD"), &sig, &sig, "Add README.md", &tree, &[])
                .unwrap();
        }

        let blame = repo.blame_file(Path::new("README.md"), None).unwrap();
        // Cannot use unwrap_err() because Blame does not implement Debug
        let result = match blame.blame_buffer(b"") {
            Ok(_) => panic!("Expected an error"),
            Err(e) => e,
        };
        assert_eq!(
            crate::Error::new(
                crate::ErrorCode::GenericError,
                crate::ErrorClass::Invalid,
                "buffer cannot be empty"
            ),
            result
        );
    }
}
