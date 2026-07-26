use crate::domain::Segment;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::Path;

pub struct Store {
    writer: File,
}

impl Store {
    pub fn open(path: &Path) -> io::Result<Self> {
        let writer = OpenOptions::new().create(true).append(true).open(path)?;
        Ok(Self { writer })
    }

    pub fn append(&mut self, segment: &Segment) -> io::Result<()> {
        let line = serde_json::to_string(segment).map_err(io::Error::other)?;
        self.writer.write_all(line.as_bytes())?;
        self.writer.write_all(b"\n")?;
        self.writer.flush()?;
        self.writer.sync_data()?;
        Ok(())
    }
}

pub fn read_all(path: &Path) -> io::Result<Vec<Segment>> {
    let content = fs::read_to_string(path)?;
    let lines: Vec<&str> = content
        .split('\n')
        .filter(|line| !line.is_empty())
        .collect();

    let mut segments = Vec::with_capacity(lines.len());
    for (i, line) in lines.iter().enumerate() {
        match serde_json::from_str::<Segment>(line) {
            Ok(segment) => segments.push(segment),
            Err(e) => {
                if i == lines.len() - 1 {
                    break;
                }
                return Err(io::Error::other(e));
            }
        }
    }
    Ok(segments)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::SpeakerTag;
    use std::fs;

    fn make_segment(id: u64) -> Segment {
        Segment {
            id,
            speaker_tag: if id.is_multiple_of(2) {
                SpeakerTag::Them
            } else {
                SpeakerTag::Me
            },
            start_ms: id * 100,
            end_ms: id * 100 + 50,
            text: format!("segment {id}"),
            mean_confidence: 0.8,
        }
    }

    #[test]
    fn appended_segments_round_trip_in_order() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("transcript.jsonl");

        let segments: Vec<Segment> = (1..=3).map(make_segment).collect();

        let mut store = super::Store::open(&path).unwrap();
        for segment in &segments {
            store.append(segment).unwrap();
        }

        let read_back = super::read_all(&path).unwrap();
        assert_eq!(read_back, segments);
    }

    #[test]
    fn truncated_final_line_is_dropped_without_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("transcript.jsonl");

        let seg1 = make_segment(1);
        let seg2 = make_segment(2);

        let mut raw = String::new();
        raw.push_str(&serde_json::to_string(&seg1).unwrap());
        raw.push('\n');
        raw.push_str(&serde_json::to_string(&seg2).unwrap());
        raw.push('\n');
        // Deliberately truncated: no closing brace, no trailing newline.
        raw.push_str(r#"{"id":3,"speaker_tag":"me","start_ms":10"#);

        fs::write(&path, raw).unwrap();

        let read_back = super::read_all(&path).unwrap();
        assert_eq!(read_back, vec![seg1, seg2]);
    }

    #[test]
    fn append_is_durable_before_the_next_append_returns() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("transcript.jsonl");

        let segment = make_segment(1);

        let mut store = super::Store::open(&path).unwrap();
        store.append(&segment).unwrap();

        let expected_line = serde_json::to_string(&segment).unwrap();
        let bytes_via_second_handle = fs::read_to_string(&path).unwrap();
        assert!(
            bytes_via_second_handle.contains(&expected_line),
            "appended Segment must be readable via an independent file handle immediately after append() returns"
        );
    }
}
