//! Damped spring used for every island animation (size, corner radius,
//! content opacity, and pop-text scale).
//! Adapted from `oyenza-studio/crates/poni-core/src/spring.rs`.

#[derive(Clone, Copy, Debug)]
pub struct Spring {
    pub value: f32,
    pub velocity: f32,
    pub target: f32,
    pub stiffness: f32,
    pub damping: f32,
}

impl Spring {
    pub fn new(value: f32, stiffness: f32, damping: f32) -> Self {
        Self {
            value,
            velocity: 0.0,
            target: value,
            stiffness,
            damping,
        }
    }

    /// Slightly under-damped spring (small overshoot), ideal for island expansion and pop text.
    pub fn bouncy(value: f32) -> Self {
        Self::new(value, 420.0, 30.0)
    }

    /// Critically damped spring (no overshoot), ideal for opacity and smooth easing.
    pub fn smooth(value: f32) -> Self {
        let k = 380.0f32;
        Self::new(value, k, 2.0 * k.sqrt())
    }

    /// Snappy pop spring for text scale (0.85 -> 1.0 with subtle elastic pop).
    pub fn pop(value: f32) -> Self {
        Self::new(value, 520.0, 32.0)
    }

    pub fn retune(&mut self, stiffness: f32, damping: f32) {
        self.stiffness = stiffness;
        self.damping = damping;
    }

    pub fn set_target(&mut self, target: f32) {
        self.target = target;
    }

    pub fn snap(&mut self, value: f32) {
        self.value = value;
        self.target = value;
        self.velocity = 0.0;
    }

    /// Advance by `dt` seconds (semi-implicit Euler with fixed 240Hz sub-steps).
    pub fn step(&mut self, dt: f32) {
        if self.is_settled() {
            self.value = self.target;
            self.velocity = 0.0;
            return;
        }
        let mut remaining = dt.clamp(0.0, 0.1);
        const H: f32 = 1.0 / 240.0;
        while remaining > 0.0 {
            let h = remaining.min(H);
            let force = -self.stiffness * (self.value - self.target) - self.damping * self.velocity;
            self.velocity += force * h;
            self.value += self.velocity * h;
            remaining -= h;
        }
        if self.is_settled() {
            self.value = self.target;
            self.velocity = 0.0;
        }
    }

    pub fn is_settled(&self) -> bool {
        self.is_settled_eps(0.05)
    }

    pub fn is_settled_eps(&self, eps: f32) -> bool {
        (self.value - self.target).abs() < eps && self.velocity.abs() < eps * 4.0
    }
}
