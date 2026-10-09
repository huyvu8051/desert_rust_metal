
/// Same `[stats]` shape the other two baselines print, minus the phases this
/// one does not have — there is no ECS to update and no snapshot to extract.
#[derive(Default)]
pub struct FramePhaseStats {
    frames: u32,
    cpu: f32,
    encode: f32,
    draws: usize,
}

impl FramePhaseStats {
    pub fn record(&mut self, cpu: f32, encode: f32, draws: usize) {
        self.frames += 1;
        self.cpu += cpu;
        self.encode += encode;
        self.draws = draws;
        if self.cpu < 2000.0 {
            return;
        }
        let n = self.frames as f32;
        println!(
            "[stats] {:.0} fps  cpu {:.2} ms = update 0.00 + extract 0.00 + encode {:.2}  draws {}",
            1000.0 / (self.cpu / n),
            self.cpu / n,
            self.encode / n,
            self.draws
        );
        *self = Self::default();
    }
}
