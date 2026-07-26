use std::fs::File;
use std::io::BufWriter;
use std::path::Path;

pub struct WavWriter {
    inner: hound::WavWriter<BufWriter<File>>,
}

impl WavWriter {
    pub fn create(path: &Path) -> std::io::Result<Self> {
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 16_000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let inner = hound::WavWriter::create(path, spec).map_err(std::io::Error::other)?;
        Ok(Self { inner })
    }

    pub fn write_samples(&mut self, samples: &[i16]) -> std::io::Result<()> {
        for &sample in samples {
            self.inner
                .write_sample(sample)
                .map_err(std::io::Error::other)?;
        }
        Ok(())
    }

    pub fn finalize(self) -> std::io::Result<()> {
        self.inner.finalize().map_err(std::io::Error::other)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wav_header_reflects_final_sample_count_after_finalize() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("out.wav");

        let samples: Vec<i16> = (0..4_000i32).map(|i| (i % 1000) as i16).collect();

        let mut writer = WavWriter::create(&path).unwrap();
        writer.write_samples(&samples).unwrap();
        writer.finalize().unwrap();

        let reader = hound::WavReader::open(&path).unwrap();
        assert_eq!(reader.duration(), samples.len() as u32);
    }
}
