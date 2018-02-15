use ::traits::IsNewtype;

use ::std::path::{Path, PathBuf};
use ::std::io::Result as IoResult;
use ::std::sync::atomic::AtomicUsize;
use ::std::sync::atomic::Ordering::SeqCst;
use ::std::sync::Arc;
use ::std::fmt;

#[derive(Clone, )]
pub(crate) struct AtomicCounter(Arc<AtomicUsize>);

impl AtomicCounter {
    pub fn new() -> Self { AtomicCounter(Arc::new(AtomicUsize::new(0))) }
    pub fn get(&self) -> usize { self.0.load(SeqCst) }
    pub fn inc(&self) -> usize { self.0.fetch_add(1, SeqCst) }
}

impl fmt::Display for AtomicCounter {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        <usize as fmt::Display>::fmt(&self.get(), f)
    }
}

/// RAII type to temporarily enter a directory.
///
/// The recommended usage is actually not to rely on the implicit destructor
/// (which panics on failure), but to instead explicitly call `.pop()`.
/// The advantage of doing so over just manually calling 'set_current_dir'
/// is the unused variable lint can help remind you to call `pop`.
///
/// Usage is highly discouraged in multithreaded contexts where
/// another thread may need to access the filesystem.
#[must_use]
pub struct PushDir(Option<PathBuf>);
pub fn push_dir<P: AsRef<Path>>(path: P) -> IoResult<PushDir> {
    let old = ::std::env::current_dir()?;
    ::std::env::set_current_dir(path)?;
    Ok(PushDir(Some(old)))
}

impl PushDir {
    /// Explicitly destroy the PushDir.
    ///
    /// This lets you handle the IO error, and has an advantage over
    /// manual calls to 'env::set_current_dir' in that the compiler will
    pub fn pop(mut self) -> IoResult<()> {
        ::std::env::set_current_dir(self.0.take().unwrap())
    }
}

impl Drop for PushDir {
    fn drop(&mut self) {
        if let Some(d) = self.0.take() {
            if let Err(e) = ::std::env::set_current_dir(d) {
                // uh oh.
                panic!("automatic popdir failed: {}", e);
            }
        }
    }
}

pub(crate) fn zip_eq<As, Bs>(a: As, b: Bs) -> ::std::iter::Zip<As::IntoIter, Bs::IntoIter>
where
    As: IntoIterator, As::IntoIter: ExactSizeIterator,
    Bs: IntoIterator, Bs::IntoIter: ExactSizeIterator,
{
    let (a, b) = (a.into_iter(), b.into_iter());
    assert_eq!(a.len(), b.len());
    a.zip(b)
}

pub(crate) fn index_of_nearest(carts: &[[f64; 3]], needle: &[f64; 3], tol: f64) -> Option<usize>
{
    use ::rsp2_array_utils::{arr_from_fn, dot};
    carts.into_iter()
        .map(|v| arr_from_fn(|k| v[k] - needle[k]))
        .map(|v: [_; 3]| dot(&v, &v))
        .enumerate()
        .filter(|&(_, sq)| sq <= tol)
        .min_by(|&(_, v1), &(_, v2)| v1.partial_cmp(&v2).expect("NaN"))
        .map(|(i, _)| i)
}

#[allow(unused)]
pub(crate) fn index_of_shortest(carts: &[[f64; 3]], tol: f64) -> Option<usize>
{ index_of_nearest(carts, &[0.0; 3], tol) }

/// Newtype for canonicalized paths
#[derive(Debug)]
pub(crate) struct CanonicalPath(Path);

unsafe impl ::traits::IsNewtype<Path> for CanonicalPath { }

impl ::std::ops::Deref for CanonicalPath {
    type Target = Path;
    fn deref(&self) -> &Path { &self.0 }
}

impl AsRef<Path> for CanonicalPath {
    fn as_ref(&self) -> &Path { &self.0 }
}

pub(crate) fn canonicalize<P: ::traits::AsPath>(path: P) -> ::Result<Box<CanonicalPath>> {
    let path = ::rsp2_fs_util::canonicalize(path.as_path())?;
    let path = path.into_boxed_path();
    Ok(CanonicalPath::wrap_box(path))
}

// Canonicalize a path where the last component need not exist
pub(crate) fn canonicalize_parent<P: ::traits::AsPath>(path: P) -> ::Result<Box<CanonicalPath>> {
    let path = ::rsp2_fs_util::canonicalize_parent(path.as_path())?;
    let path = path.into_boxed_path();
    Ok(CanonicalPath::wrap_box(path))
}

pub(crate) use self::lockfile::{LockfilePath, LockfileGuard};
mod lockfile {
    use ::Result;
    use ::std::fs::{OpenOptions};
    use ::std::io;
    use ::rsp2_fs_util as fsx;
    use ::std::path::{Path, PathBuf};

    /// Handle with methods for creating a lockfile without race conditions.
    #[derive(Debug, Clone)]
    pub struct LockfilePath(pub PathBuf);

    /// RAII guard for a lockfile.
    #[derive(Debug)]
    pub struct LockfileGuard(PathBuf);

    #[allow(dead_code)]
    impl LockfilePath {
        pub fn try_lock(&self) -> Result<Option<LockfileGuard>> {
            let path = ::util::canonicalize_parent(&self.0)?.to_path_buf();
            // 'create_new' is the magic sauce for avoiding race conditions
            let result = OpenOptions::new().write(true)
                                           .create_new(true)
                                           .open(&path);
            match result {
                Err(e) => {
                    match e.kind() {
                        io::ErrorKind::AlreadyExists => Ok(None),
                        _ => bail!(e),
                    }
                },
                Ok(_) => Ok(Some(LockfileGuard(self.0.clone()))),
            }
        }

        pub fn lock(&self) -> Result<Option<LockfileGuard>> {
            let mut lock = self.try_lock()?;
            while lock.is_none() {
                ::std::thread::sleep(Default::default());
                lock = self.try_lock()?;
            }
            Ok(lock)
        }

        pub fn path(&self) -> &Path
        { &self.0 }
    }

    #[allow(dead_code)]
    impl LockfileGuard {
        pub fn drop(self) -> Result<()>
        { self._drop() }

        fn _drop(&self) -> Result<()>
        { Ok(fsx::remove_file(&self.0)?) }

        #[allow(dead_code)]
        pub fn path(&self) -> &Path
        { &self.0 }
    }

    #[allow(dead_code)]
    impl Drop for LockfileGuard {
        fn drop(&mut self) {
            let _ = self._drop();
        }
    }
}

extension_trait!{
    <'a> pub ArgMatchesExt<'a> for ::clap::ArgMatches<'a> {
        // For when the value ought to exist because it was 'required(true)'
        // (and therefore clap would have panicked if it were missing)
        fn expect_value_of(&self, s: &str) -> String
        { self.value_of(s).unwrap_or_else(|| panic!("BUG! ({} was required)", s)).into() }

        fn expect_values_of(&self, s: &str) -> Vec<String>
        { self.values_of(s).unwrap_or_else(|| panic!("BUG! ({} was required)", s)).map(Into::into).collect() }
    }
}
