use super::Accumulator;

// Four independent lanes permit SLP without LLVM's unsafe floating-point
// reassociation flags. The supplied operators inline before vectorization.
#[inline]
fn reduce4(xs: &[f64], init: f64, identity: f64, op: impl Fn(f64, f64) -> f64, term: impl Fn(f64) -> f64) -> f64 {
    if xs.is_empty() {
        return init;
    }
    let mut a = [init, identity, identity, identity];
    let mut chunks = xs.chunks_exact(4);
    for c in &mut chunks {
        for j in 0..4 {
            a[j] = op(a[j], term(c[j]));
        }
    }
    let mut s = op(op(a[0], a[1]), op(a[2], a[3]));
    for &x in chunks.remainder() {
        s = op(s, term(x));
    }
    s
}

impl Accumulator for f64 {
    const ZERO: Self = 0.0;
    const ONE: Self = 1.0;
    #[inline]
    fn from_f64(x: f64) -> Self {
        x
    }
    #[inline]
    fn from_i64(x: i64) -> Self {
        x as f64
    }
    #[inline]
    fn to_f64(self) -> f64 {
        self
    }
    #[inline]
    fn sqrt(self) -> Self {
        self.sqrt()
    }
    #[inline]
    fn sum_from(self, xs: &[f64]) -> Self {
        reduce4(xs, self, 0.0, |a, b| a + b, |x| x)
    }
    #[inline]
    fn prod_from(self, xs: &[f64]) -> Self {
        reduce4(xs, self, 1.0, |a, b| a * b, |x| x)
    }
    #[inline]
    fn sum_dev(self, xs: &[f64]) -> Self {
        reduce4(xs, 0.0, 0.0, |a, b| a + b, |x| x - self)
    }
    #[inline]
    fn sum_sq_dev(self, xs: &[f64]) -> Self {
        reduce4(
            xs,
            0.0,
            0.0,
            |a, b| a + b,
            |x| {
                let d = x - self;
                d * d
            },
        )
    }
    #[inline]
    fn sum_cross_dev(self, xs: &[f64], ym: Self, ys: &[f64]) -> Self {
        assert_eq!(xs.len(), ys.len());
        let mut a = [0.0; 4];
        let mut xx = xs.chunks_exact(4);
        let mut yy = ys.chunks_exact(4);
        for (x, y) in xx.by_ref().zip(yy.by_ref()) {
            for j in 0..4 {
                a[j] += (x[j] - self) * (y[j] - ym);
            }
        }
        let mut s = (a[0] + a[1]) + (a[2] + a[3]);
        for (&x, &y) in xx.remainder().iter().zip(yy.remainder()) {
            s += (x - self) * (y - ym);
        }
        s
    }
    #[inline]
    fn cumsum_from(self, xs: &[f64], out: &mut [f64]) -> Self {
        assert!(out.len() >= xs.len());
        let mut s = self;
        for (&x, y) in xs.iter().zip(out) {
            s += x;
            *y = s;
        }
        s
    }
    #[inline]
    fn cumprod_from(self, xs: &[f64], out: &mut [f64]) -> Self {
        assert!(out.len() >= xs.len());
        let mut s = self;
        for (&x, y) in xs.iter().zip(out) {
            s *= x;
            *y = s;
        }
        s
    }
    #[inline]
    fn add_each(acc: &mut [Self], xs: &[f64]) {
        assert!(acc.len() >= xs.len());
        for (a, &x) in acc.iter_mut().zip(xs) {
            *a += x;
        }
    }
}
