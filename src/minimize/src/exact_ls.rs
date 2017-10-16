error_chain!{
    types {
        Error, ErrorKind, ResultExt, LsResult;
    }
    errors {
        BadBound(b: f64) {
            description("An input bound was too extreme")
            display("The input bound was too extreme: {}", b)
        }
        GsBadValue(endvals: (f64, f64), value: f64) {
            description("Golden search encountered value larger than endpoints")
            display("Golden search encountered value larger than endpoints: {:?} vs {}", endvals, value)
        }
        NoMinimum {
            description("The function appears to have no minimum")
            display("The function appears to have no minimum", )
        }
        FunctionOutput(b: f64) {
            description("The function produced an inscrutible value")
            display("The function produced an inscrutible value: {}", b)
        }
    }
}

#[derive(Debug, Copy, Clone, PartialEq, PartialOrd)]
pub struct Value(pub f64);
#[derive(Debug, Copy, Clone, PartialEq, PartialOrd)]
pub struct Slope(pub f64);
#[derive(Debug, Copy, Clone, PartialEq, PartialOrd)]
pub struct ValueBound { pub alpha: f64, pub value: f64 }
#[derive(Debug, Copy, Clone, PartialEq, PartialOrd)]
pub struct SlopeBound { pub alpha: f64, pub slope: f64 }
#[derive(Debug, Copy, Clone, PartialEq, PartialOrd)]
pub struct Bound { pub alpha: f64, pub value: f64, pub slope: f64 }

pub type Interval = (f64, f64);
pub type SlopeInterval = (SlopeBound, SlopeBound);
pub type ValueFn<'a, E> = FnMut(f64) -> Result<Value, E> + 'a;
pub type SlopeFn<'a, E> = FnMut(f64) -> Result<Slope, E> + 'a;
pub type OneDeeFn<'a, E> = FnMut(f64) -> Result<(Value, Slope), E> + 'a;

fn check_mirroring_assumption(x0: f64) -> LsResult<()> {
    // Assumption:
    //
    // Given an IEEE-754 floating point number of any precision
    // for which (2*x) is finite (x may be subnormal):
    //
    //      2 * x - x == x
    //
    // This has been validated by brute force for all f32
    // on x86_64 architecture.
    //
    // This assumption allows us to take a function evaluated
    // at 'x0' and change its argument to '2*x0 - x', knowing
    // that the value at 'x0' (and more importantly, the sign
    // of the slope) has been identically preserved.
    ensure!(2.0 * x0 - x0 == x0, ErrorKind::BadBound(x0));
    Ok(())
}

pub fn linesearch<E, F>(
    from: f64,
    initial_step: f64,
    mut compute: F,
) -> LsResult<Result<SlopeBound, E>>
where F: FnMut(f64) -> Result<Slope, E>
{
    // early wrapping:
    //  - SlopeBound for internal use
    //  - Detect nonsensical slopes
    //  - Result<Slope, Result<TheirError, OurError>> for easy short-circuiting
    let compute = move |alpha| {
        let slope = compute(alpha).map_err(Ok)?;
        ensure!(slope.0.is_finite(), Err(ErrorKind::FunctionOutput(slope.0).into()));
        trace!("LS-iter:  a: {:<23e}  s: {:<23e}", alpha, slope.0);
        Ok(SlopeBound { alpha, slope: slope.0 })
    };

    // make it possible to conditionally wrap the closure into another.
    let mut compute: Box<FnMut(f64) -> Result<SlopeBound, Result<E, Error>>>
        = Box::new(compute);

    nest_err(|| {
        let mut a = compute(from)?;
        if a.slope > 0.0 {
            check_mirroring_assumption(a.alpha).map_err(Err)?;
            let center = a.alpha;
            compute = Box::new(move |alpha|
                compute(2.0 * center - alpha)
                    .map(|SlopeBound { slope, .. }|
                        SlopeBound { alpha, slope: -slope })
            );
            a.slope *= -1.0;
        };
        let b = compute(from + initial_step)?;

        let (a, b) = find_initial((a, b), &mut *compute)?;
        let bound = bisect((a, b), &mut *compute)?;
        trace!("LS-exit:  a: {:<23e}  v: {:<23e}", bound.alpha, bound.slope);
        Ok(bound)
    })
}

fn find_initial<E>(
    (a, mut b): SlopeInterval,
    compute: &mut FnMut(f64) -> Result<SlopeBound, Result<E, Error>>,
) -> Result<SlopeInterval, Result<E, Error>>
{
    assert!(a.slope <= 0.0);
    while b.slope < 0.0 {
        // double the interval width
        let new_alpha = b.alpha + (b.alpha - a.alpha);
        ensure!(new_alpha.is_finite(), Err(ErrorKind::NoMinimum.into()));
        b = compute(new_alpha)?;
    }
    Ok((a, b))
}

fn bisect<E>(
    (mut lo, mut hi): SlopeInterval,
    compute: &mut FnMut(f64) -> Result<SlopeBound, E>,
) -> Result<SlopeBound, E>
{
    assert!(lo.alpha <= hi.alpha);
    loop {
        // We do allow both endpoints to have zero slope.
        assert!(lo.slope <= 0.0);
        assert!(hi.slope >= 0.0);

        let alpha = 0.5 * (lo.alpha + hi.alpha);
        if !(lo.alpha < alpha && alpha < hi.alpha) {
            return Ok(lo);
        }

        let bound = compute(alpha)?;

        // NOTE: If slope is uniformly zero, we'll shrink down to just 'lo'.
        match bound.slope >= 0.0 {
            true => hi = bound,
            false => lo = bound,
        }
    }
}

// Revelations:
//  1. In common implementations of the algorithm (such as those on wikipedia)
//     the values of the function at the endpoints are never used.
//     Hence **it is only necessary to save one y value.**
//     However, we save more because we don't trust the function's accuracy.
//  2. TECHNICALLY the step function doesn't even even need to use phi;
//      one could record 'b' and derive the second endpoint as 'c = d - b + a'.
//     But I don't know if that is numerically stable, so we will do what
//     the wikipedia implementations do and recompute b and c every iter.
pub fn golden<E, F>(
    interval: (f64, f64),
    mut compute: F,
// NOTE: cannot return a bound due to issue mentioned in body
) -> LsResult<Result<f64, E>>
where F: FnMut(f64) -> Result<Value, E>
{
    nest_err(|| {
        // early wrapping:
        //  - ValueBound for internal use
        //  - Result<Value, Result<TheirError, OurError>> for easy short-circuiting
        let mut compute = move |alpha| {
            let value = compute(alpha).map_err(Ok)?;
            ensure!(value.0.is_finite(), Err(ErrorKind::FunctionOutput(value.0).into()));
            trace!("GS-iter:  a: {:<23e}  v: {:<23e}", alpha, value.0);
            Ok(ValueBound { alpha, value: value.0 })
        };

        let phi: f64 = (1.0 + 5f64.sqrt()) / 2.0;
        let get_mid_xs = |a, d| {
            let dist = (d - a) / (1.0 + phi);
            (a + dist, d - dist)
        };

        let (mut state, mut history) = {
            // endpoints. (note: we allow d.alpha < a.alpha)
            let a = compute(interval.0)?;
            let d = compute(interval.1)?;

            // inner point closer to a
            let b = compute(get_mid_xs(a.alpha, d.alpha).0)?;

            let history = vec![a, d, b];
            ((a, b, d), history)
        };

        loop {
            let (a, mut b, d) = state;

            // Stop when it is dead obvious that the value is no longer numerically reliable.
            if b.value > a.value.min(d.value) { break; }

            // re-adjust b, purportedly to avoid systematic issues with precision
            // that can cause infinite loops. (I dunno. ask whoever edits wikipedia)
            //
            // NOTE: Technically this desynchronizes the alpha of our Bounds from
            //  the values, so at the end we cannot return a bound.
            let (b_alpha, c_alpha) = get_mid_xs(a.alpha, d.alpha);
            b.alpha = b_alpha;

            let c = compute(c_alpha)?;

            history.push(c);
            state = match b.value < c.value {
                true => (c, b, a),
                false => (b, c, d),
            }
        }
        //history.sort_on_key(|bound| NotNaN::new(bound.alpha).unwrap());
        let (_a, b, _d) = state;
        Ok(b.alpha)
    })
}

// (NOTE: takes an IIFE so that ? can be used inside of it)
fn nest_err<A, B, C, F>(f: F)-> Result<Result<A, B>, C>
where F: FnOnce() -> Result<A, Result<B, C>>
{
    match f() {
        Ok(x) => Ok(Ok(x)),
        Err(Ok(e)) => Ok(Err(e)),
        Err(Err(e)) => Err(e),
    }
}
