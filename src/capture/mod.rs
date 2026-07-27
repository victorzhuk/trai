pub mod pw_record;
pub mod source_list;
pub mod wav_writer;

use wav_writer::WavWriter;

/// Reads raw little-endian i16 PCM bytes from `source` until EOF, decoding
/// them into samples and forwarding each decoded chunk to both `wav` and
/// `on_samples` (in production, `Segmenter::push_samples`).
pub fn tee_pcm_to_wav<R: std::io::Read>(
    mut source: R,
    wav: &mut WavWriter,
    mut on_samples: impl FnMut(&[i16]),
) -> std::io::Result<()> {
    const CHUNK_BYTES: usize = 4096;
    let mut byte_buf = [0u8; CHUNK_BYTES];
    let mut carry: Option<u8> = None;
    let mut samples = Vec::with_capacity(CHUNK_BYTES / 2 + 1);

    loop {
        let read = source.read(&mut byte_buf)?;
        if read == 0 {
            break;
        }
        samples.clear();
        let mut bytes = &byte_buf[..read];

        if let Some(lo) = carry.take() {
            if let Some(&hi) = bytes.first() {
                samples.push(i16::from_le_bytes([lo, hi]));
                bytes = &bytes[1..];
            } else {
                carry = Some(lo);
                continue;
            }
        }

        let mut chunks = bytes.chunks_exact(2);
        for pair in &mut chunks {
            samples.push(i16::from_le_bytes([pair[0], pair[1]]));
        }
        if let Some(&b) = chunks.remainder().first() {
            carry = Some(b);
        }

        if !samples.is_empty() {
            wav.write_samples(&samples)?;
            on_samples(&samples);
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn tee_pcm_to_wav_reaches_both_wav_writer_and_sample_sink() {
        let input_samples: Vec<i16> = (-2000..2000).step_by(3).collect();
        let raw_bytes: Vec<u8> = input_samples.iter().flat_map(|s| s.to_le_bytes()).collect();

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tee.wav");
        let mut wav = WavWriter::create(&path).unwrap();

        let mut collected: Vec<i16> = Vec::new();
        tee_pcm_to_wav(Cursor::new(raw_bytes), &mut wav, |chunk| {
            collected.extend_from_slice(chunk);
        })
        .unwrap();

        assert_eq!(
            collected, input_samples,
            "segmenter side must get every sample"
        );

        wav.finalize().unwrap();

        let mut reader = hound::WavReader::open(&path).unwrap();
        assert_eq!(reader.duration(), input_samples.len() as u32);
        let wav_samples: Vec<i16> = reader
            .samples::<i16>()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(wav_samples, input_samples, "WAV side must get every sample");
    }
}
