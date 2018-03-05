//! Filesystem-based API.
//!
//! This API is centered around two types of objects:
//!
//! - Wrappers around directory types (which are generic in the
//!   directory type and may contain TempDirs, PathBufs, or even
//!   borrowed &Paths), representing a directory where some
//!   computation was run.
//! - Builders for configuring the next computation.
//!
//! The directory wrapper types allow the user to be able to
//! explicitly "keep" a phonopy output directory, and potentially
//! even reuse it in a future computation. The inability to do these
//! things was the biggest limitation of the previous API, which never
//! exposed its own temporary directories.

use ::{Error, Result, IoResult, ErrorKind};
use ::As3;

use super::{Conf, DispYaml, SymmetryYaml, QPositions, Args, OtherSettings};
use ::traits::{AsPath, HasTempDir, Save, Load};

use ::rsp2_structure_io::poscar;
use ::std::io::prelude::*;
use ::std::process::Command;
use ::std::path::{Path, PathBuf};
use ::rsp2_fs_util::mv;
use ::rsp2_tempdir::TempDir;

use ::rsp2_kets::Basis;
use ::rsp2_fs_util::{open, create, open_text, copy, hard_link};
use ::rsp2_structure::{ElementStructure, Element};
use ::rsp2_structure::{FracRot, FracTrans, FracOp};
use ::rsp2_phonopy_io::npy;

use ::rsp2_array_types::V3;

use ::slice_of_array::prelude::*;

const THZ_TO_WAVENUMBER: f64 = 33.35641;

#[derive(Debug, Clone)]
pub struct Builder {
    symprec: Option<f64>,
    conf: Conf,
    more: OtherSettings,
}

impl Default for Builder {
    fn default() -> Self {
        Builder {
            symprec: None,
            conf: Default::default(),
            more: OtherSettings {
                use_sparse_sets: false,
            },
        }
    }
}

const FNAME_OTHER_SETTINGS: &'static str = "rsp2-phonopy.json";

impl Builder {
    pub fn new() -> Self
    { Default::default() }

    pub fn symmetry_tolerance(mut self, x: f64) -> Self
    { self.symprec = Some(x); self }

    pub fn conf<K: AsRef<str>, V: AsRef<str>>(mut self, key: K, value: V) -> Self
    { self.conf.0.insert(key.as_ref().to_owned(), value.as_ref().to_owned()); self }

    /// Extend with configuration lines from a phonopy .conf file.
    /// If the file defines a value that was already set, the new
    ///  value from the file will take precedence.
    // FIXME: Result<Self>... oh, THAT's why the general recommendation is for &mut Self
    #[allow(unused)]
    pub fn conf_from_file<R: BufRead>(self, file: R) -> Result<Self>
    {Ok({
        let mut me = self;
        for (key, value) in ::rsp2_phonopy_io::conf::read(file)? {
            me = me.conf(key, value);
        }
        me
    })}

    pub fn supercell_dim<V: As3<u32>>(self, dim: V) -> Self
    {
        self.conf("DIM", {
            let (a, b, c) = dim.as_3();
            format!("{} {} {}", a, b, c)
        })
    }

    pub fn use_sparse_sets(mut self, value: bool) -> Self
    { self.more.use_sparse_sets = value; self }

    fn args_from_settings(&self) -> Args
    {
        let mut out = vec![];
        if let Some(tol) = self.symprec {
            out.push(format!("--tolerance"));
            out.push(format!("{:e}", tol));
        }
        out.into()
    }
}

impl Builder {
    pub fn displacements(
        &self,
        structure: &ElementStructure,
    ) -> Result<DirWithDisps<TempDir>>
    {Ok({
        let dir = TempDir::new("rsp2")?;
        {
            let dir = dir.path();
            trace!("Displacement dir: '{}'...", dir.display());

            let extra_args = self.args_from_settings();
            self.conf.save(dir.join("disp.conf"))?;
            poscar::dump(create(dir.join("POSCAR"))?, "blah", &structure)?;
            extra_args.save(dir.join("disp.args"))?;
            self.more.save(dir.join(FNAME_OTHER_SETTINGS))?;

            trace!("Calling phonopy for displacements...");
            {
                let mut command = Command::new("phonopy");
                command
                    .args(&extra_args.0)
                    .arg("disp.conf")
                    .arg("--displacement")
                    .current_dir(&dir);

                log_stdio_and_wait(command, None)?;
            }
        };

        DirWithDisps::from_existing(dir)?
    })}

    // FIXME: Should return a new DirWithSymmetry type.
    // FIXME: The 'symmetry-test' was the only binary shim that used this and
    //        I removed it,  was removed during a refactor but
    //        this is nontrivial.  I'd rather re-add the shim.
    #[allow(unused)]
    pub fn symmetry(
        &self,
        structure: &ElementStructure,
    ) -> Result<(Vec<FracOp>)>
    {Ok({
        let tmp = TempDir::new("rsp2")?;
        let tmp = tmp.path();
        trace!("Entered '{}'...", tmp.display());

        self.conf.save(tmp.join("phonopy.conf"))?;

        poscar::dump(create(tmp.join("POSCAR"))?, "blah", &structure)?;

        trace!("Calling phonopy for symmetry...");
        check_status(Command::new("phonopy")
            .args(self.args_from_settings().0)
            .arg("phonopy.conf")
            .arg("--sym")
            .current_dir(&tmp)
            .stdout(create(tmp.join("symmetry.yaml"))?)
            .status()?)?;

        trace!("Done calling phonopy");

        // check if input structure was primitive
        {
            let prim = poscar::load(open(tmp.join("PPOSCAR"))?)?;

            let ratio = structure.lattice().volume() / prim.lattice().volume();
            let ratio = round_checked(ratio, 1e-4)?;

            // sorry, supercells are just not supported... yet.
            //
            // (In the future we may be able to instead return an object
            //  which will allow the spacegroup operators of the primitive
            //  to be applied in meaningful ways to the superstructure.)
            ensure!(ratio == 1, ErrorKind::NonPrimitiveStructure);
        }

        let yaml = SymmetryYaml::load(tmp.join("symmetry.yaml"))?;
        yaml.space_group_operations.into_iter()
            .map(|op| Ok({
                let rotation = FracRot::new(&op.rotation);
                let translation = FracTrans::from_floats(&op.translation)?;
                FracOp::new(&rotation, &translation)
            }))
            .collect::<Result<_>>()?
    })}
}

// Declares a type whose body is hidden behind an Option,
// which can be taken to "poison" (consume) the value.
//
// This is a design pattern for &mut Self-based builders.
macro_rules! declare_poison_pair {
    (
        generics: { $($generics:tt)* }
        where: { $($bounds:tt)* }
        type: {
            #[derive($($derive:ident),*)]
            pub struct $Type:ident<...>(Option<_>);
            struct $Impl:ident<...> { $($body:tt)* }
        }
        poisoned: $poisoned:block
    ) => {
        #[derive($($derive),*)]
        pub struct $Type<$($generics)*>(Option<$Impl<$($generics)*>>)
        where $($bounds)*;

        #[derive($($derive),*)]
        struct $Impl<$($generics)*>
        where $($bounds)*
        { $($body)* }

        impl<$($generics)*> $Type<$($generics)*> where $($bounds)*
        {
            /// modify self if not poisoned
            fn inner_mut(&mut self) -> &mut $Impl<$($generics)*>
            { match self.0 {
                Some(ref mut inner) => inner,
                None => $poisoned,
            }}

            /// poisons self
            fn into_inner(&mut self) -> $Impl<$($generics)*>
            { match self.0.take() {
                Some(inner) => inner,
                None => $poisoned,
            }}
        }

    }
}

/// Represents a directory with the following data:
/// - `POSCAR`: The input structure
/// - `disp.yaml`: Phonopy file with displacements
/// - configuration settings which impact the selection of displacements
///   - `--tol`, `--dim`
///
/// Generally, the next step is to supply the force sets, turning this
/// into a DirWithForces.
///
/// # Note
///
/// Currently, the implementation is rather optimistic that files in
/// the directory have not been tampered with since its creation.
/// As a result, some circumstances which probably should return `Error`
/// may instead cause a panic, or may not be detected as early as possible.
#[derive(Debug, Clone)]
pub struct DirWithDisps<P: AsPath> {
    pub(crate) dir: P,
    // These are cached in memory from `disp.yaml` due to the likelihood
    // that code using `DirWithDisps` will need them.
    pub(crate) superstructure: ElementStructure,
    pub(crate) displacements: Vec<(usize, V3)>,
    pub(crate) settings: OtherSettings,
}

impl<P: AsPath> DirWithDisps<P> {
    pub fn from_existing(dir: P) -> Result<Self>
    {Ok({
        for name in &[
            "POSCAR",
            "disp.yaml",
            "disp.conf",
            "disp.args",
            FNAME_OTHER_SETTINGS,
        ] {
            let path = dir.as_path().join(name);
            ensure!(path.exists(),
                ErrorKind::MissingFile("DirWithDisps", dir.as_path().to_owned(), name.to_string()));
        }

        trace!("Parsing disp.yaml...");
        let DispYaml {
            displacements, structure: superstructure
        } = Load::load(dir.as_path().join("disp.yaml"))?;
        let superstructure = superstructure.try_map_metadata_into(|d| {
            match Element::from_symbol(&d.symbol[..]) {
                Some(e) => Ok::<_, Error>(e),
                None => bail!("invalid symbol in disp.yaml: {:?}", d.symbol),
            }
        })?;
        let settings = Load::load(dir.as_path().join(FNAME_OTHER_SETTINGS))?;

        DirWithDisps { dir, superstructure, displacements, settings }
    })}

    #[allow(unused)]
    pub fn superstructure(&self) -> &ElementStructure
    { &self.superstructure }
    pub fn displacements(&self) -> &[(usize, V3)]
    { &self.displacements }

    /// Due to the "StreamingIterator problem" this iterator must return
    /// clones of the structure... though it seems unlikely that this cost
    /// is anything to worry about compared to the cost of computing the
    /// forces on said structure.
    pub fn displaced_structures<'a>(&'a self) -> Box<Iterator<Item=ElementStructure> + 'a>
    { Box::new({
        use ::rsp2_phonopy_io::disp_yaml::apply_displacement;
        self.displacements
            .iter()
            .map(move |&disp| apply_displacement(&self.superstructure, disp))
    })}

    /// Write FORCE_SETS (or SPARSE_SETS) to create a `DirWithForces`
    /// (which may serve as a template for one or more band computations).
    ///
    /// This variant creates a new temporary directory.
    pub fn make_force_dir<Vs>(self, forces: Vs) -> Result<DirWithForces<TempDir>>
    where
        Vs: IntoIterator,
        <Vs as IntoIterator>::IntoIter: ExactSizeIterator,
        <Vs as IntoIterator>::Item: AsRef<[V3]>,
    {Ok({
        let out = TempDir::new("rsp2")?;
        trace!("Force sets dir: '{}'...", out.path().display());

        self.make_force_dir_in_dir(forces, out)?
    })}

    /// Write FORCE_SETS (or SPARSE_SETS) to create a `DirWithForces`
    /// (which may serve as a template for one or more band computations).
    ///
    /// This variant uses the specified path.
    pub fn make_force_dir_in_dir<Vs, Q>(self, forces: Vs, path: Q) -> Result<DirWithForces<Q>>
    where
        Q: AsPath,
        Vs: IntoIterator,
        <Vs as IntoIterator>::IntoIter: ExactSizeIterator,
        <Vs as IntoIterator>::Item: AsRef<[V3]>,
    {Ok({
        self.prepare_force_dir(forces, path.as_path())?;
        DirWithForces::from_existing(path)?
    })}

    /// Creates the files expected by DirWithForces::from_existing
    fn prepare_force_dir<Vs>(self, forces: Vs, path: &Path) -> Result<()>
    where
        Vs: IntoIterator,
        <Vs as IntoIterator>::IntoIter: ExactSizeIterator,
        <Vs as IntoIterator>::Item: AsRef<[V3]>,
    {Ok({
        let disp_dir = self.path();

        for name in &["POSCAR", "disp.yaml", "disp.conf", "disp.args", FNAME_OTHER_SETTINGS] {
            copy(disp_dir.join(name), path.join(name))?;
        }

        match self.settings.use_sparse_sets {
            false => {
                trace!("Writing FORCE_SETS...");
                ::rsp2_phonopy_io::force_sets::write(
                    create(path.join("FORCE_SETS"))?,
                    &self.displacements,
                    forces,
                )?;
            },
            true => {
                trace!("Writing SPARSE_SETS...");
                ::rsp2_phonopy_io::sparse_sets::write(
                    create(path.join("SPARSE_SETS"))?,
                    &self.displacements,
                    forces,
                )?;
            },
        }
    })}
}

/// Represents a directory with the following data:
/// - `POSCAR`: The input structure
/// - `FORCE_SETS`: Phonopy file with forces for displacements
/// - configuration settings which impact the selection of displacements
///   - `--tol`, `--dim`
/// - OtherSettings
///
/// It may also contain a force constants file.
///
/// Generally, one produces a `DirWithBands` from this by selecting some
/// q-points to sample.
///
/// # Note
///
/// Currently, the implementation is rather optimistic that files in
/// the directory have not been tampered with since its creation.
/// As a result, some circumstances which probably should return `Error`
/// may instead cause a panic, or may not be detected as early as possible.
#[derive(Debug, Clone)]
pub struct DirWithForces<P: AsPath> {
    dir: P,
    settings: OtherSettings,
    cache_force_constants: bool,
}

impl<P: AsPath> DirWithForces<P> {
    pub fn from_existing(dir: P) -> Result<Self>
    {Ok({
        let settings = OtherSettings::load(dir.as_path().join(FNAME_OTHER_SETTINGS))?;

        // Sanity check
        for name in &[
            "POSCAR",
            settings.force_sets_filename(),
            "disp.args",
            "disp.conf",
        ] {
            let path = dir.as_path().join(name);
            ensure!(path.exists(),
                ErrorKind::MissingFile("DirWithForces", dir.as_path().to_owned(), name.to_string()));
        }
        DirWithForces { dir, settings, cache_force_constants: true }
    })}

    #[allow(unused)]
    pub fn structure(&self) -> Result<ElementStructure>
    { Ok(poscar::load(open_text(self.path().join("POSCAR"))?)?) }

    /// Enable/disable caching of force constants.
    ///
    /// When enabled (the default), force constants are copied back from
    /// the first successful `DirWithBands` created, and are reused in
    /// subsequent band computations. (these copies are hard links if possible)
    #[allow(unused)]
    pub fn cache_force_constants(&mut self, b: bool) -> &mut Self
    { self.cache_force_constants = b; self }

    /// Compute bands in a temp directory.
    ///
    /// Returns an object used to configure the computation.
    pub fn build_bands(&self) -> BandsBuilder<P>
    { BandsBuilder::init(self) }
}

declare_poison_pair! {
    generics: {'p, P}
    where: {
        P: AsPath + 'p,
    }
    type: {
        #[derive(Debug, Clone)]
        pub struct BandsBuilder<...>(Option<_>);
        struct BandsBuilderImpl<...> {
            dir_with_forces: &'p DirWithForces<P>,
            eigenvectors: bool,
        }
    }
    poisoned: { panic!("This BandsBuilder has already been used!"); }
}

impl<'p, P: AsPath> BandsBuilder<'p, P> {
    fn init(dir_with_forces: &'p DirWithForces<P>) -> Self
    { BandsBuilder(Some(BandsBuilderImpl {
        dir_with_forces,
        eigenvectors: false,
    })) }

    pub fn eigenvectors(&mut self, b: bool) -> &mut Self
    { self.inner_mut().eigenvectors = b; self }

    pub fn compute(&mut self, q_points: &[V3]) -> Result<DirWithBands<TempDir>>
    {Ok({
        let me = self.into_inner();
        let dir = TempDir::new("rsp2")?;

        let fc_filename = "force_constants.hdf5";
        {
            let src = me.dir_with_forces.as_path();
            let dir = dir.as_path();

            for name in &["POSCAR", "disp.conf", "disp.args", FNAME_OTHER_SETTINGS] {
                copy(src.join(name), dir.join(name))?;
            }
            {
                let name = me.dir_with_forces.settings.force_sets_filename();
                copy_or_link(src.join(name), dir.join(name))?;
            }

            if src.join(fc_filename).exists() {
                copy_or_link(src.join(fc_filename), dir.join(fc_filename))?;
            }

            QPositions(q_points.into()).save(dir.join("q-positions.json"))?;

            // band.conf
            {
                // Carry over settings from displacements.
                let Conf(mut conf) = Load::load(dir.join("disp.conf"))?;

                // Append a dummy qpoint so that each of our points begin a line segment.
                conf.insert("BAND".to_string(), band_string(q_points) + " 0 0 0");
                // NOTE: using 1 band point does result in a (currently inconsequential) divide-by-zero
                //       warning in phonopy, but cuts the filesize of band output by half compared to 2
                //       points. Considering how large the band output is, I'll take my chances!
                //          - ML
                conf.insert("BAND_POINTS".to_string(), "1".to_string());
                conf.insert("EIGENVECTORS".to_string(), match me.eigenvectors {
                    true => ".TRUE.".to_string(),
                    false => ".FALSE.".to_string(),
                });
                Conf(conf).save(dir.join("band.conf"))?;
            }

            trace!("Calling phonopy for bands...");
            {
                let mut command = Command::new("phonopy");
                command
                    .args(Args::load(dir.join("disp.args"))?.0)
                    .arg("band.conf")
                    .arg("--fc-format=hdf5")
                    .arg("--band-format=hdf5")
                    .arg(match dir.join(fc_filename).exists() {
                        true => "--readfc",
                        false => "--writefc",
                    })
                    .current_dir(&dir);

                log_stdio_and_wait(command, None)?;
            }

            trace!("Converting bands...");
            {
                let mut command = Command::new("python3");
                command.current_dir(&dir);

                // ayyyyup.
                log_stdio_and_wait(command, Some("
import numpy as np
import h5py

band = h5py.File('band.hdf5')
np.save('eigenvector.npy', band['eigenvector'])
np.save('eigenvalue.npy', band['frequency'])
np.save('q-position.npy', band['path'])
np.save('q-distance.npy', band['distance'])

del band
import os; os.unlink('band.hdf5')
".to_string()))?;
            }

            if me.dir_with_forces.cache_force_constants {
                cache_link(dir.join(fc_filename), src.join(fc_filename))?;
            }
        }

        DirWithBands::from_existing(dir)?
    })}
}

/// Represents a directory with the following data:
/// - input structure
/// - q-points
/// - eigenvalues (and possibly eigenvectors)
/// - OtherSettings
///
/// # Note
///
/// Currently, the implementation is rather optimistic that files in
/// the directory have not been tampered with since its creation.
/// As a result, some circumstances which probably should return `Error`
/// may instead cause a panic, or may not be detected as early as possible.
#[derive(Debug, Clone)]
pub struct DirWithBands<P: AsPath> {
    settings: OtherSettings,
    dir: P,
}


impl<P: AsPath> DirWithBands<P> {
    pub fn from_existing(dir: P) -> Result<Self>
    {Ok({
        let settings = OtherSettings::load(dir.as_path().join(FNAME_OTHER_SETTINGS))?;

        // Sanity check
        for name in &[
            "POSCAR",
            settings.force_sets_filename(),
            "eigenvalue.npy",
            "q-positions.json",
        ] {
            let path = dir.as_path().join(name);
            ensure!(path.exists(),
                ErrorKind::MissingFile("DirWithBands", dir.as_path().to_owned(), name.to_string()));
        }

        DirWithBands { settings, dir }
    })}

    pub fn structure(&self) -> Result<ElementStructure>
    { Ok(poscar::load(open_text(self.path().join("POSCAR"))?)?) }

    pub fn q_positions(&self) -> Result<Vec<V3>>
    { Ok(QPositions::load(self.path().join("q-positions.json"))?.0) }

    /// This will be `None` if `.eigenvectors(true)` was not set prior
    /// to the band computation.
    pub fn eigenvectors(&self) -> Result<Option<Vec<Basis>>>
    {Ok({
        let path = self.path().join("eigenvector.npy");
        if path.exists() {
            trace!("Reading eigenvectors...");
            Some(npy::read_eigenvector_npy(open(path)?)?)
        } else { None }
    })}

    pub fn eigenvalues(&self) -> Result<Vec<Vec<f64>>>
    {Ok({
        use ::rsp2_slice_math::{v};
        trace!("Reading eigenvectors...");
        let file = open(self.path().join("eigenvalue.npy"))?;
        npy::read_eigenvalue_npy(file)?
            .into_iter()
            .map(|evs| (v(evs) * THZ_TO_WAVENUMBER).0)
            .collect()
    })}
}

//-----------------------------

fn band_string(ks: &[V3]) -> String
{ ks.flat().iter().map(|x| x.to_string()).collect::<Vec<_>>().join(" ") }

fn round_checked(x: f64, tol: f64) -> Result<i32>
{Ok({
    let r = x.round();
    ensure!((r - x).abs() < tol, "not nearly integral: {}", x);
    r as i32
})}

pub(crate) fn log_stdio_and_wait(
    mut cmd: ::std::process::Command,
    stdin: Option<String>,
) -> Result<()>
{Ok({
    use ::std::process::Stdio;
    use ::std::io::{BufRead, BufReader};

    if stdin.is_some() {
        cmd.stdin(Stdio::piped());
    }

    debug!("$ {:?}", cmd);

    let mut child = cmd
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;

    if let Some(text) = stdin {
        child.stdin.take().unwrap().write_all(text.as_bytes())?;
    }

    let stdout_worker = {
        let f = BufReader::new(child.stdout.take().unwrap());
        ::std::thread::spawn(move || -> Result<()> {Ok({
            for line in f.lines() {
                ::stdout::log(&(line?[..]));
            }
        })})
    };

    let stderr_worker = {
        let f = BufReader::new(child.stderr.take().unwrap());
        ::std::thread::spawn(move || -> Result<()> {Ok({
            for line in f.lines() {
                ::stderr::log(&(line?[..]));
            }
        })})
    };

    check_status(child.wait()?)?;

    let _ = stdout_worker.join();
    let _ = stderr_worker.join();
})}

fn check_status(status: ::std::process::ExitStatus) -> Result<()>
{Ok({
    ensure!(status.success(), ErrorKind::PhonopyFailed(status));
})}

// Wrapper around `hard_link` which:
// - falls back to copying if the destination is on another filesystem.
// - does not fail if the destination exists (it is simply left behind)
//
// The use case is where this may be used on many identical source files
// for the same destination, possibly concurrently, in order to cache the
// file for further reuse.
pub fn cache_link<P: AsRef<Path>, Q: AsRef<Path>>(src: P, dest: Q) -> Result<()>
{
    let (src, dest) = (src.as_ref(), dest.as_ref());
    hard_link(src, dest)
        .map(|_| ())
        .or_else(|_| Ok({
            // if the file already existed, the link will have failed.
            // Check this before continuing because we don't want to
            //   potentially overwrite a link with a copy.
            if dest.exists() {
                return Ok(());
            }

            // assume the error was due to being on a different filesystem.
            // (Even if not, we will probably just get the same error)
            copy(src, dest)?;
        }))
}

// Like `cache_link` except it fails if the destination exists.
pub fn copy_or_link<P: AsRef<Path>, Q: AsRef<Path>>(src: P, dest: Q) -> Result<()>
{
    let (src, dest) = (src.as_ref(), dest.as_ref());
    hard_link(src, dest)
        .map(|_| ())
        .or_else(|_| Ok({
            // assume that, if the error was due to anything other than cross-device linking,
            // then we'll get the same error again when we try to copy.
            copy(src, dest)?;
        }))
}

//-----------------------------

impl_dirlike_boilerplate!{
    type: {DirWithDisps<_>}
    member: self.dir
    other_members: [self.displacements, self.settings, self.superstructure]
}

impl_dirlike_boilerplate!{
    type: {DirWithForces<_>}
    member: self.dir
    other_members: [self.cache_force_constants, self.settings]
}

impl_dirlike_boilerplate!{
    type: {DirWithBands<_>}
    member: self.dir
    other_members: [self.settings]
}

#[cfg(test)]
#[deny(unused)]
mod tests {
    use super::*;

    // check use cases I need to work
    #[allow(unused)]
    fn things_expected_to_impl_aspath() {
        use ::std::rc::Rc;
        use ::std::sync::Arc;
        panic!("This is a compiletest; it should not be executed.");

        // clonable TempDirs
        let x: DirWithBands<TempDir> = panic!();
        let x = x.map_dir(Rc::new);
        let _ = x.clone();

        // sharable TempDirs
        let x: DirWithBands<TempDir> = panic!();
        let x = x.map_dir(Arc::new);
        let _: &(Send + Sync) = &x;

        // erased types, for conditional deletion
        let _: DirWithBands<Box<AsPath>> = x.map_dir(|e| Box::new(e) as _);
    }
}
