//! Double-double arithmetic (about 106 bits), the working precision of the correctly rounded
//! `sin`, `cos`, `log` and `pow`.

#[derive(Clone, Copy)]
pub(crate) struct Dd(pub f64, pub f64);

impl Dd {
    pub(crate) fn neg(self) -> Dd {
        Dd(-self.0, -self.1)
    }
}

pub(crate) fn two_sum(a: f64, b: f64) -> Dd {
    let s = a + b;
    let bb = s - a;
    Dd(s, (a - (s - bb)) + (b - bb))
}

pub(crate) fn quick_two_sum(a: f64, b: f64) -> Dd {
    let s = a + b;
    Dd(s, b - (s - a))
}

pub(crate) fn add(a: Dd, b: Dd) -> Dd {
    let s = two_sum(a.0, b.0);
    let t = two_sum(a.1, b.1);
    let s = quick_two_sum(s.0, s.1 + t.0);
    quick_two_sum(s.0, s.1 + t.1)
}

pub(crate) fn mul(a: Dd, b: Dd) -> Dd {
    let p = a.0 * b.0;
    quick_two_sum(p, a.0.mul_add(b.0, -p) + (a.0 * b.1 + a.1 * b.0))
}

pub(crate) fn mul_f64(a: Dd, b: f64) -> Dd {
    let p = a.0 * b;
    quick_two_sum(p, a.0.mul_add(b, -p) + a.1 * b)
}

pub(crate) fn div_f64(a: Dd, b: f64) -> Dd {
    let q = a.0 / b;
    let r = add(a, mul_f64(Dd(q, 0.0), -b));
    quick_two_sum(q, r.0 / b)
}

pub(crate) fn sub(a: Dd, b: Dd) -> Dd {
    add(a, b.neg())
}

/// `a / b` in double-double (three quotient steps).
pub(crate) fn div(a: Dd, b: Dd) -> Dd {
    let q1 = a.0 / b.0;
    let r = sub(a, mul_f64(b, q1));
    let q2 = r.0 / b.0;
    let r = sub(r, mul_f64(b, q2));
    let q3 = r.0 / b.0;
    add(quick_two_sum(q1, q2), Dd(q3, 0.0))
}

/// `a · f` for a power of two `f` (exact while nothing leaves the normal range).
pub(crate) fn scale(a: Dd, f: f64) -> Dd {
    Dd(a.0 * f, a.1 * f)
}
