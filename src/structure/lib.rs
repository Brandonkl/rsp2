extern crate rsp2_array_utils;
extern crate rsp2_array_types;
#[cfg(test)] #[macro_use] extern crate rsp2_assert_close;


extern crate ordered_float;
extern crate slice_of_array;
#[macro_use] extern crate log;
#[macro_use] extern crate itertools;
#[macro_use] extern crate error_chain;
#[macro_use] extern crate lazy_static;
#[cfg(test)] extern crate rand;

error_chain!{
    errors {
        BadPerm {
            description("Tried to construct an invalid permutation.")
            display("Tried to construct an invalid permutation.")
        }
        BadPart {
            description("Tried to construct an invalid partition.")
            display("Tried to construct an invalid partition.")
        }
        BigDisplacement(d: f64) {
            description("Suspiciously large movement between supercell images."),
            display("Suspiciously large movement between supercell images: {:e}", d),
        }
        IntPrecisionError(d: f64) {
            description("Poor precision for float approximation of integer."),
            display("Not nearly an integer: {}", d),
        }
        NonEquivalentLattice(a_binv: [[f64; 3]; 3]) {
            description("The new lattice is not equivalent to the original."),
            display("The new lattice is not equivalent to the original. (A B^-1 = {:?})", a_binv),
        }
    }
}
mod errors {
    pub use ::{Result, Error, ErrorKind, ResultExt};
}

#[cfg(test)]
macro_rules! assert_matches {
    ($pat:pat, $expr:expr,)
    => { assert_matches!($pat, $expr) };
    ($pat:pat, $expr:expr)
    => { assert_matches!($pat, $expr, "actual {:?}", $expr) };
    ($pat:pat, $expr:expr, $($arg:expr),+ $(,)*)
    => {
        match $expr {
            $pat => {},
            _ => panic!(
                "assertion failed: {} ({})",
                stringify!(assert_matches!($pat, $expr)),
                format_args!($($arg),+))
        }
    };
}

pub mod helper {
    pub use ::oper::part::composite_perm_for_part_lifo;
}

pub mod supercell {
    pub use ::algo::supercell::{
        diagonal,
        diagonal_with,
        OwnedMetas,
        SupercellToken,
    };
}

mod core;
mod algo;
mod oper;
mod util;
mod element;

//---------------------------
// public reexports; API

pub use ::oper::perm::{Perm, Permute};
pub use ::oper::part::{Part, Parted, Partition, Unlabeled};
pub use ::core::lattice::Lattice;
pub use ::core::coords::Coords;
pub use ::core::structure::{Structure, CoordStructure, ElementStructure};

pub use ::element::Element;

pub use algo::layer::{find_layers, Layers, LayersPerUnitCell};

// yuck. would rather not expose this yet
pub use ::oper::symmops::{FracRot, FracTrans, FracOp};

pub use ::algo::find_perm::dumb_symmetry_test;

pub use element::consts as consts;

