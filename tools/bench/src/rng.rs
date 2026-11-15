//! xorshift64: the same numbers on every platform, for workloads drawn
//! with arithmetic and square roots only.

pub struct Rng(pub u64);

impl Rng {
    /// Uniform in [0, 1).
    pub fn next(&mut self) -> f64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 11) as f64 / (1u64 << 53) as f64
    }

    /// Uniform on the unit sphere (rejection from the cube: no sin or cos).
    pub fn direction(&mut self) -> [f64; 3] {
        loop {
            let v = [0; 3].map(|_| 2.0 * self.next() - 1.0);
            let r2: f64 = v.iter().map(|x| x * x).sum();
            if r2 > 1e-6 && r2 <= 1.0 {
                let r = r2.sqrt();
                return v.map(|x| x / r);
            }
        }
    }
}
