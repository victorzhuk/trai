use crate::domain::Segment;
use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::Path;

#[derive(Debug, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Record {
    Segment(Segment),
    Translation {
        segment_id: u64,
        text: String,
        #[serde(default)]
        degraded: bool,
    },
}

pub struct Store {
    writer: File,
}

impl Store {
    pub fn open(path: &Path) -> io::Result<Self> {
        let writer = OpenOptions::new().create(true).append(true).open(path)?;
        Ok(Self { writer })
    }

    pub fn append(&mut self, segment: &Segment) -> io::Result<()> {
        self.append_record(&Record::Segment(segment.clone()))
    }

    pub fn append_record(&mut self, record: &Record) -> io::Result<()> {
        let line = serde_json::to_string(record).map_err(io::Error::other)?;
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

    let mut segments: Vec<Segment> = Vec::with_capacity(lines.len());
    let mut index_by_id: HashMap<u64, usize> = HashMap::new();
    let mut translations_by_id: HashMap<u64, (String, bool)> = HashMap::new();

    for (i, line) in lines.iter().enumerate() {
        match serde_json::from_str::<Record>(line) {
            Ok(Record::Segment(mut segment)) => {
                if let Some((text, degraded)) = translations_by_id.get(&segment.id) {
                    segment.translation = Some(text.clone());
                    segment.degraded = *degraded;
                }
                index_by_id.insert(segment.id, segments.len());
                segments.push(segment);
            }
            Ok(Record::Translation {
                segment_id,
                text,
                degraded,
            }) => {
                translations_by_id.insert(segment_id, (text.clone(), degraded));
                if let Some(index) = index_by_id.get(&segment_id) {
                    segments[*index].translation = Some(text);
                    segments[*index].degraded = degraded;
                }
            }
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
            translation: None,
            degraded: false,
            translation_error: None,
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
    fn translation_records_merge_into_segments() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("transcript.jsonl");

        let s1 = make_segment(1);
        let s2 = make_segment(2);

        let mut store = super::Store::open(&path).unwrap();
        store.append(&s1).unwrap();
        store
            .append_record(&Record::Translation {
                segment_id: 1,
                text: "hola".to_string(),
                degraded: false,
            })
            .unwrap();
        store.append(&s2).unwrap();
        store
            .append_record(&Record::Translation {
                segment_id: 2,
                text: "bonjour".to_string(),
                degraded: false,
            })
            .unwrap();

        let read_back = super::read_all(&path).unwrap();
        assert_eq!(read_back.len(), 2);
        assert_eq!(read_back[0].translation.as_deref(), Some("hola"));
        assert_eq!(read_back[1].translation.as_deref(), Some("bonjour"));
    }

    #[test]
    fn degraded_translation_record_merges_into_segment() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("transcript.jsonl");

        let segment = make_segment(1);

        let mut store = super::Store::open(&path).unwrap();
        store.append(&segment).unwrap();
        store
            .append_record(&Record::Translation {
                segment_id: 1,
                text: "hola".to_string(),
                degraded: true,
            })
            .unwrap();

        let read_back = super::read_all(&path).unwrap();
        assert_eq!(read_back.len(), 1);
        assert_eq!(read_back[0].translation.as_deref(), Some("hola"));
        assert!(read_back[0].degraded);
    }

    #[test]
    fn old_format_segment_record_without_degraded_key_defaults_to_false() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("transcript.jsonl");

        let raw = concat!(
            r#"{"kind":"segment","id":1,"speaker_tag":"me","start_ms":100,"end_ms":150,"#,
            r#""text":"segment 1","mean_confidence":0.8}"#,
            "\n"
        );
        fs::write(&path, raw).unwrap();

        let read_back = super::read_all(&path).unwrap();
        assert_eq!(read_back.len(), 1);
        assert_eq!(read_back[0].id, 1);
        assert_eq!(read_back[0].text, "segment 1");
        assert!(!read_back[0].degraded);
    }

    #[test]
    fn old_format_translation_record_without_degraded_key_defaults_to_false() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("transcript.jsonl");

        let segment = make_segment(1);

        let mut raw = String::new();
        raw.push_str(&serde_json::to_string(&Record::Segment(segment.clone())).unwrap());
        raw.push('\n');
        raw.push_str(r#"{"kind":"translation","segment_id":1,"text":"hola"}"#);
        raw.push('\n');

        fs::write(&path, raw).unwrap();

        let read_back = super::read_all(&path).unwrap();
        assert_eq!(read_back.len(), 1);
        assert_eq!(read_back[0].translation.as_deref(), Some("hola"));
        assert!(!read_back[0].degraded);
    }

    #[test]
    fn missing_translation_record_is_treated_as_none() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("transcript.jsonl");

        let segment = make_segment(1);
        let mut store = super::Store::open(&path).unwrap();
        store.append(&segment).unwrap();

        let read_back = super::read_all(&path).unwrap();
        assert_eq!(read_back.len(), 1);
        assert_eq!(read_back[0].translation, None);
    }

    #[test]
    fn truncated_final_line_is_dropped_without_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("transcript.jsonl");

        let seg1 = make_segment(1);
        let seg2 = make_segment(2);

        let mut raw = String::new();
        raw.push_str(&serde_json::to_string(&Record::Segment(seg1.clone())).unwrap());
        raw.push('\n');
        raw.push_str(&serde_json::to_string(&Record::Segment(seg2.clone())).unwrap());
        raw.push('\n');
        // Deliberately truncated: no closing quote, no trailing newline.
        raw.push_str(r#"{"kind":"segment","id":3,"speaker_tag":"me","start_ms":10"#);

        fs::write(&path, raw).unwrap();

        let read_back = super::read_all(&path).unwrap();
        assert_eq!(read_back, vec![seg1, seg2]);
    }

    #[test]
    fn interleaved_translation_records_merge_by_segment_id() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("transcript.jsonl");

        let segment1 = make_segment(1);
        let segment2 = make_segment(2);

        let mut raw = String::new();
        raw.push_str(&serde_json::to_string(&Record::Segment(segment1.clone())).unwrap());
        raw.push('\n');
        raw.push_str(&serde_json::to_string(&Record::Segment(segment2.clone())).unwrap());
        raw.push('\n');
        raw.push_str(
            &serde_json::to_string(&Record::Translation {
                segment_id: 2,
                text: "seg2 translation".to_string(),
                degraded: false,
            })
            .unwrap(),
        );
        raw.push('\n');
        raw.push_str(
            &serde_json::to_string(&Record::Translation {
                segment_id: 1,
                text: "seg1 translation".to_string(),
                degraded: false,
            })
            .unwrap(),
        );
        raw.push('\n');

        fs::write(&path, raw).unwrap();

        let read_back = super::read_all(&path).unwrap();
        assert_eq!(read_back.len(), 2);
        assert_eq!(
            read_back[0].translation.as_deref(),
            Some("seg1 translation")
        );
        assert_eq!(
            read_back[1].translation.as_deref(),
            Some("seg2 translation")
        );
    }

    #[test]
    fn append_is_durable_before_the_next_append_returns() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("transcript.jsonl");

        let segment = make_segment(1);

        let mut store = super::Store::open(&path).unwrap();
        store.append(&segment).unwrap();

        let expected_line = serde_json::to_string(&Record::Segment(segment.clone())).unwrap();
        let bytes_via_second_handle = fs::read_to_string(&path).unwrap();
        assert!(
            bytes_via_second_handle.contains(&expected_line),
            "appended Segment must be readable via an independent file handle immediately after append() returns"
        );
    }
}
