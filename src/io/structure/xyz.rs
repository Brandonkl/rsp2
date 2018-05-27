use ::FailResult;
use ::std::io::prelude::*;

use ::rsp2_structure::{Element};

use ::rsp2_array_types::V3;

//--------------------------------------------------------------------------------------
// public API

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Xyz<
    Title = String,
    Carts = Vec<V3>,
    Elements = Vec<Element>,
> {
    pub title: Title,
    pub carts: Carts,
    pub elements: Elements,
}

impl<Title, Carts, Elements> Xyz<Title, Carts, Elements>
where
    Title: AsRef<str>,
    Carts: AsRef<[V3]>,
    Elements: AsRef<[Element]>,
{
    /// Writes an XYZ frame to an open file.
    ///
    /// You can freely call this multiple times on the same file
    /// to write an animation, since XYZ animations are simply
    /// concatenated XYZ files.
    pub fn to_writer(&self, mut w: impl Write) -> FailResult<()> {
        dump(&mut w, self.title.as_ref(), self.carts.as_ref(), self.elements.as_ref())
    }
}

//--------------------------------------------------------------------------------------
// implementation

fn dump(w: &mut Write, title: &str, carts: &[V3], types: &[Element]) -> FailResult<()>
{
    assert!(!title.contains("\n"));
    assert!(!title.contains("\r"));
    assert_eq!(carts.len(), types.len());

    writeln!(w, "{}", carts.len())?;
    writeln!(w, "{}", title)?;
    for (V3([x, y, z]), typ) in carts.iter().zip(types) {
        writeln!(w, " {:>2} {} {} {}", typ.symbol(), x, y, z)?;
    }

    Ok(())
}
