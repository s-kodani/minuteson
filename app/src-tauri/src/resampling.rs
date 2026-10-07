use std::f64::consts::PI;

const PHASES: usize = 256;

// The same windowed-sinc filter serves analysis and meeting playback. Every
// output sample uses an absolute rational phase, so seeks and packet boundaries
// do not accumulate a different phase. Callers retain both sides of the halo.
pub(crate) struct Filter {
    pub(crate) half: i64,
    output_rate: u32,
    kernels: Vec<Vec<f64>>,
}

impl Filter {
    pub(crate) fn new(input_rate: u32, output_rate: u32) -> Self {
        assert!(input_rate > 0 && output_rate > 0);
        let cutoff = (f64::from(output_rate) / f64::from(input_rate)).min(1.0) * 0.95;
        let half = (16.0 / cutoff).ceil() as i64;
        let kernels = (0..=PHASES)
            .map(|phase| {
                let fraction = phase as f64 / PHASES as f64;
                let mut kernel: Vec<f64> = (-half..=half)
                    .map(|tap| {
                        let distance = tap as f64 - fraction;
                        let x = distance * cutoff;
                        let sinc = if x.abs() < 1e-12 {
                            1.0
                        } else {
                            (PI * x).sin() / (PI * x)
                        };
                        let window = if distance.abs() > half as f64 {
                            0.0
                        } else {
                            0.42 + 0.5 * (PI * distance / half as f64).cos()
                                + 0.08 * (2.0 * PI * distance / half as f64).cos()
                        };
                        cutoff * sinc * window
                    })
                    .collect();
                let sum: f64 = kernel.iter().sum();
                for value in &mut kernel {
                    *value /= sum;
                }
                kernel
            })
            .collect();
        Self {
            half,
            output_rate,
            kernels,
        }
    }

    pub(crate) fn sample(&self, input: &[f32], index: u64, rate: u32) -> f32 {
        self.sample_from(input, index, rate, 0)
    }

    pub(crate) fn sample_from(
        &self,
        input: &[f32],
        index: u64,
        rate: u32,
        first_frame: u64,
    ) -> f32 {
        self.sample_at(
            input,
            u128::from(index) * u128::from(rate),
            u128::from(self.output_rate),
            first_frame,
        )
    }

    pub(crate) fn sample_at(
        &self,
        input: &[f32],
        numerator: u128,
        denominator: u128,
        first_frame: u64,
    ) -> f32 {
        let base = (numerator / denominator) as i128 - i128::from(first_frame);
        let phase =
            (((numerator % denominator) * PHASES as u128 + denominator / 2) / denominator) as usize;
        self.kernels[phase]
            .iter()
            .enumerate()
            .map(|(tap, weight)| {
                let source =
                    (base + tap as i128 - i128::from(self.half)).clamp(0, input.len() as i128 - 1);
                f64::from(input[source as usize]) * weight
            })
            .sum::<f64>() as f32
    }
}
