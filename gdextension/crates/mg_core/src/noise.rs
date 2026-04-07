pub trait NoiseStrategy: Send + Sync {
    fn generate(&self, x: f64, y: f64, detail_level: u32) -> f64;
    fn name(&self) -> &'static str {
        "NoiseStrategy"
    }
}
