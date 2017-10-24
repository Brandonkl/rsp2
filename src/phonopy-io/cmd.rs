use ::errors::*;
use ::Displacements;
use ::DispYaml;

use ::rsp2_structure::{CoordStructure, ElementStructure};
use ::rsp2_structure::{FracRot, FracTrans, FracOp};

use ::tempdir::TempDir;
use ::std::process::Command;
use ::std::io::prelude::*;
use ::std::fs;
use ::std::fs::File;
use ::std::path::Path;
use ::std::collections::HashMap;

fn write_conf<W>(mut w: W, conf: &HashMap<String, String>) -> Result<()>
where W: Write,
{
    for (key, val) in conf {
        ensure!(key.bytes().all(|c| c != b'='), "'=' in conf key");
        writeln!(w, "{} = {}", key, val)?
    }
    Ok(())
}

#[derive(Debug, Clone, Default)]
pub struct Builder {
    symprec: Option<f64>,
    conf: HashMap<String, String>,
}


impl Builder {
    pub fn new() -> Self
    { Default::default() }

    pub fn symmetry_tolerance(mut self, x: f64) -> Self
    {
        self.symprec = Some(x);
        self
    }

    pub fn conf<K: AsRef<str>, V: AsRef<str>>(mut self, key: K, value: V) -> Self
    {
        self.conf.insert(key.as_ref().to_owned(), value.as_ref().to_owned());
        self
    }

    fn args_from_settings(&self) -> Vec<String>
    {
        let mut out = vec![];
        if let Some(tol) = self.symprec {
            out.push(format!("--tolerance"));
            out.push(format!("{:e}", tol));
        }
        out
    }
}

impl Builder {
    // FIXME computation functions and builder setters in same namespace,
    //       this is weird/confusing
    pub fn displacements(
        &self,
        structure: ElementStructure,
    ) -> Result<(CoordStructure, Displacements, TempDir)>
    {
        use ::rsp2_structure_io::poscar;

        let tmp = TempDir::new("rsp2")?;
        let (displacements, superstructure) = {

            let tmp = tmp.path();
            trace!("Entered '{}'...", tmp.display());

            write_conf(
                File::create(tmp.join("phonopy.conf"))?,
                &self.conf,
            )?;

            poscar::dump(
                File::create(tmp.join("POSCAR"))?,
                "blah",
                &structure,
            )?;

            trace!("Calling phonopy for displacements...");
            {
                let mut command = Command::new("phonopy");
                command
                    .args(self.args_from_settings())
                    .arg("phonopy.conf")
                    .arg("--displacement")
                    .current_dir(&tmp);

                log_stdio_and_wait(command)?;
            }

            trace!("Parsing disp.yaml...");
            let DispYaml {
                displacements, structure: superstructure
            } = ::disp_yaml::read(File::open(tmp.join("disp.yaml"))?)?;

            (displacements, superstructure)
        };

        Ok((superstructure.map_metadata_into(|_| ()), displacements, tmp))
    }

    pub fn gamma_eigensystem(
        mut self: &Self,
        force_sets: Vec<Vec<[f64; 3]>>,
        disp_dir: &AsRef<Path>,
    ) -> Result<(Vec<f64>, Vec<Vec<[f64; 3]>>)>
    {Ok({
        use ::slice_of_array::prelude::*;

        let disp_dir = disp_dir.as_ref();

        let tmp = TempDir::new("rsp2")?;
        let tmp = tmp.path();
        trace!("Entered '{}'...", tmp.display());

        let mut me = self.clone();
        let _guard = &mut self; // prevent accidental usage of self
        me = me.conf("BAND", "0 0 0 1 0 0");
        me = me.conf("BAND_POINTS", "2");
        write_conf(File::create(tmp.join("phonopy.conf"))?, &me.conf)?;

        fs::copy(disp_dir.join("POSCAR"), tmp.join("POSCAR"))?;

        trace!("Parsing disp.yaml...");
        let DispYaml {
            displacements, structure: superstructure,
        } = ::disp_yaml::read(File::open(disp_dir.join("disp.yaml"))?)?;

        trace!("Writing FORCE_SETS...");
        ::force_sets::write(
            File::create(tmp.join("FORCE_SETS"))?,
            &superstructure,
            &displacements,
            &force_sets,
        )?;

        trace!("Calling phonopy for eigenvectors...");
        {
            let mut command = Command::new("phonopy");
            command
                .args(me.args_from_settings())
                .arg("phonopy.conf")
                .arg("--eigenvectors")
                .env("EIGENVECTOR_NPY_HACK", "1")
                .current_dir(&tmp);

            log_stdio_and_wait(command)?;
        }

        trace!("Reading eigenvectors...");
        let bases = ::npy::read_eigenvector_npy(File::open(tmp.join("eigenvector.npy"))?)?;
        trace!("Reading eigenvalues...");
        let freqs = ::npy::read_eigenvalue_npy(File::open(tmp.join("eigenvalue.npy"))?)?;

        // eigensystem at first kpoint (gamma)
        let basis = bases.into_iter().next().unwrap();
        let freqs = freqs.into_iter().next().unwrap();

        trace!("Getting real..."); // :P
        let evecs = basis.iter().map(|ev| Ok(
            ev.iter().map(|c| {
                // gamma kets are real
                ensure!(c.imag == 0.0, "non-real eigenvector");
                Ok(c.real)
            }).collect::<Result<Vec<_>>>()?.nest().to_vec()
        )).collect::<Result<_>>()?;
        trace!("Done computing eigensystem");
        (freqs, evecs)
    })}

    pub fn symmetry(
        &self,
        structure: &ElementStructure,
    ) -> Result<(Vec<FracOp>)>
    {Ok({
        use ::rsp2_structure_io::poscar;
        use ::filetypes::symmetry_yaml;

        let tmp = TempDir::new("rsp2")?;
        let tmp = tmp.path();
        trace!("Entered '{}'...", tmp.display());

        write_conf(File::create(tmp.join("phonopy.conf"))?, &self.conf)?;

        poscar::dump(
            File::create(tmp.join("POSCAR"))?,
            "blah",
            &structure,
        )?;

        trace!("Calling phonopy for symmetry...");
        check_status(Command::new("phonopy")
            .args(self.args_from_settings())
            .arg("phonopy.conf")
            .arg("--sym")
            .current_dir(&tmp)
            .stdout(File::create(tmp.join("symmetry.yaml"))?)
            .status()?)?;

        trace!("Done calling phonopy");

        // check if input structure was primitive
        {
            let prim = poscar::load(File::open(tmp.join("PPOSCAR"))?)?;

            let ratio = structure.lattice().volume() / prim.lattice().volume();
            let ratio = round_checked(ratio, 1e-4)?;

            // sorry, supercells are just not supported... yet.
            //
            // (In the future we may be able to instead return an object
            //  which will allow the spacegroup operators of the primitive
            //  to be applied in meaningful ways to the superstructure.)
            ensure!(ratio == 1, ErrorKind::NonPrimitiveStructure);
        }

        let yaml = symmetry_yaml::read(File::open(tmp.join("symmetry.yaml"))?)?;
        yaml.space_group_operations.into_iter()
            .map(|op| Ok({
                let rotation = FracRot::new(&op.rotation);
                let translation = FracTrans::from_floats(&op.translation)?;
                FracOp::new(&rotation, &translation)
            }))
            .collect::<Result<_>>()?
    })}
}

fn round_checked(x: f64, tol: f64) -> Result<i32>
{Ok({
    let r = x.round();
    ensure!((r - x).abs() < tol, "not nearly integral: {}", x);
    r as i32
})}

fn check_status(status: ::std::process::ExitStatus) -> Result<()>
{Ok({
    ensure!(status.success(), ErrorKind::PhonopyFailed(status));
})}

fn log_stdio_and_wait(mut cmd: ::std::process::Command) -> Result<()>
{Ok({
    use ::std::process::Stdio;
    use ::std::io::{BufRead, BufReader};

    debug!("$ {:?}", cmd);

    let mut child = cmd
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;

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
