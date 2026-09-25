use crate::domain::Segment;
use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufRead, Write};
use std::os::unix::fs::OpenOptionsExt;
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
    // File length up to the last record that fully landed. A write_all
    // that failed partway leaves a fragment past this mark; the next
    // append truncates it before writing, since that record never
    // fully landed and is about to be rewritten anyway.
    clean_len: u64,
    // A completed write whose sync_data failed: the row is in the file,
    // so retrying would duplicate it, but its durability is unproven.
    // Sticky until `take_durability_error` surfaces it to the caller.
    durable_failure: Option<io::Error>,
}

impl Store {
    pub fn open(path: &Path) -> io::Result<Self> {
        // Transcripts hold meeting content: owner-only from creation.
        let writer = OpenOptions::new()
            .create(true)
            .read(true)
            .append(true)
            .mode(0o600)
            .open(path)?;
        // A previous run may have died mid-write_all, leaving an
        // unterminated fragment at EOF. A fragment is a partial record
        // that never fully landed, so truncating it back to the last
        // newline loses nothing and keeps every later append on its own
        // line.
        let mut len = writer.metadata()?.len();
        if len > 0 {
            use std::io::{Read, Seek, SeekFrom};
            let mut handle = writer.try_clone()?;
            let mut scan_from = len;
            let mut last_newline: Option<u64> = None;
            const CHUNK: u64 = 4096;
            while last_newline.is_none() && scan_from > 0 {
                let chunk_len = CHUNK.min(scan_from);
                scan_from -= chunk_len;
                handle.seek(SeekFrom::Start(scan_from))?;
                let mut buf = vec![0u8; chunk_len as usize];
                handle.read_exact(&mut buf)?;
                if let Some(pos) = buf.iter().rposition(|&b| b == b'\n') {
                    last_newline = Some(scan_from + pos as u64 + 1);
                }
            }
            let clean_len = last_newline.unwrap_or(0);
            if clean_len != len {
                handle.set_len(clean_len)?;
                handle.seek(SeekFrom::End(0))?;
                len = clean_len;
            }
        }
        Ok(Self {
            writer,
            clean_len: len,
            durable_failure: None,
        })
    }

    // Drains the sticky durability failure from a sync_data that failed
    // after a completed write. The rows are already in the file; this
    // only reports that their durability was never proven.
    pub fn take_durability_error(&mut self) -> Option<io::Error> {
        self.durable_failure.take()
    }

    pub fn append(&mut self, segment: &Segment) -> io::Result<()> {
        self.append_record(&Record::Segment(segment.clone()))
    }

    pub fn append_record(&mut self, record: &Record) -> io::Result<()> {
        let mut line = serde_json::to_string(record).map_err(io::Error::other)?;
        line.push('\n');
        {
            use std::io::{Seek, SeekFrom};
            if self.writer.metadata()?.len() > self.clean_len {
                // A previously failed write_all left a fragment at EOF.
                // It never fully landed, so truncate it back to the last
                // clean record rather than terminating it into a
                // malformed line.
                self.writer.set_len(self.clean_len)?;
                self.writer.seek(SeekFrom::End(0))?;
            }
        }
        match self.writer.write_all(line.as_bytes()) {
            Ok(()) => {
                // The record is now entirely in the file. A File keeps
                // no userspace buffer, so the only remaining failure is
                // sync_data, which questions durability, not the
                // append: reporting Err here would make callers retry
                // and write a duplicate row. The failure is instead
                // held on the Store and surfaced through
                // `take_durability_error` so no durability loss is
                // silent.
                self.clean_len += line.len() as u64;
                if let Err(e) = self.writer.sync_data() {
                    self.durable_failure = Some(e);
                }
                Ok(())
            }
            Err(e) => Err(e),
        }
    }
}

pub fn read_all(path: &Path) -> io::Result<Vec<Segment>> {
    let content = fs::read_to_string(path)?;
    // The only malformed line a healthy Store can produce is a torn
    // append: a single unterminated fragment at EOF (the next append
    // terminates it, read_all sees it as the last line). Any other
    // parse failure is real corruption and must be an explicit error,
    // not a silent skip.
    let ends_with_newline = content.ends_with('\n');
    let lines: Vec<&str> = content
        .split('\n')
        .filter(|line| !line.is_empty())
        .collect();

    let mut segments: Vec<Segment> = Vec::with_capacity(lines.len());
    let mut index_by_id: HashMap<u64, usize> = HashMap::new();
    let mut translations_by_id: HashMap<u64, (String, bool)> = HashMap::new();

    for (position, line) in lines.iter().enumerate() {
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
                let torn_fragment = !ends_with_newline && position == lines.len() - 1;
                if torn_fragment {
                    continue;
                }
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!(
                        "corrupt transcript record at line {} ({} readable records precede it): {e}",
                        position + 1,
                        segments.len()
                    ),
                ));
            }
        }
    }

    Ok(segments)
}

// Row count for the history listing: streamed line scan matching the
// record-type tag, not a full JSON parse of every record. Translation
// records are excluded; the listing counts transcript rows.
pub fn count_segment_lines(path: &Path) -> io::Result<usize> {
    let reader = io::BufReader::new(File::open(path)?);
    let mut count = 0;
    for line in reader.lines() {
        if line?.starts_with(r#"{"kind":"segment""#) {
            count += 1;
        }
    }
    Ok(count)
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
            source_language: None,
            translation: None,
            degraded: false,
            state: crate::domain::SegmentState::Ready,
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
    fn source_language_round_trips_and_is_omitted_when_absent() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("transcript.jsonl");

        let detected = Segment {
            source_language: Some("ru".to_string()),
            ..make_segment(1)
        };
        let unknown = make_segment(2);

        let mut store = super::Store::open(&path).unwrap();
        store.append(&detected).unwrap();
        store.append(&unknown).unwrap();
        drop(store);

        let raw = fs::read_to_string(&path).unwrap();
        let (first, second) = raw.split_once('\n').unwrap();
        assert!(first.contains(r#""source_language":"ru""#));
        assert!(
            !second.contains("source_language"),
            "a Segment with no source language must not carry the key at all"
        );

        let read_back = super::read_all(&path).unwrap();
        assert_eq!(read_back[0].source_language.as_deref(), Some("ru"));
        assert_eq!(read_back[1].source_language, None);
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
    fn append_after_a_torn_write_truncates_the_fragment_and_stays_readable() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("transcript.jsonl");

        let seg1 = make_segment(1);
        let seg2 = make_segment(2);

        let mut store = super::Store::open(&path).unwrap();
        store.append(&seg1).unwrap();

        // Simulate the EOF state a failed write_all leaves behind: a
        // partial record with no terminating newline.
        {
            use std::io::Write as _;
            let mut handle = fs::OpenOptions::new().append(true).open(&path).unwrap();
            handle.write_all(br#"{"kind":"seg"#).unwrap();
        }

        store.append(&seg2).unwrap();

        let read_back = super::read_all(&path).unwrap();
        assert_eq!(read_back, vec![seg1, seg2]);
        assert_eq!(
            fs::read_to_string(&path).unwrap().matches('\n').count(),
            2,
            "the fragment must be truncated, not terminated into a malformed line"
        );
    }

    #[test]
    fn reopen_truncates_an_unterminated_fragment_left_by_a_previous_run() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("transcript.jsonl");

        let seg1 = make_segment(1);
        let seg2 = make_segment(2);

        {
            let mut store = super::Store::open(&path).unwrap();
            store.append(&seg1).unwrap();
        }
        // Crash mid-write: a partial record without its newline.
        {
            use std::io::Write as _;
            let mut handle = fs::OpenOptions::new().append(true).open(&path).unwrap();
            handle
                .write_all(br#"{"kind":"segment","id":3,"text":"par"#)
                .unwrap();
        }

        // Reopen must inspect the last byte and repair the fragment so
        // the next append never fuses onto it.
        let mut store = super::Store::open(&path).unwrap();
        store.append(&seg2).unwrap();

        let read_back = super::read_all(&path).unwrap();
        assert_eq!(read_back, vec![seg1, seg2]);
        assert_eq!(
            fs::read_to_string(&path).unwrap().matches('\n').count(),
            2,
            "exactly two terminated records, no fused line"
        );
    }

    #[test]
    fn reopen_with_fresh_file_and_after_clean_close_stays_clean() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("transcript.jsonl");

        let seg1 = make_segment(1);
        let seg2 = make_segment(2);

        let mut store = super::Store::open(&path).unwrap();
        store.append(&seg1).unwrap();
        drop(store);

        let mut store = super::Store::open(&path).unwrap();
        store.append(&seg2).unwrap();

        assert_eq!(super::read_all(&path).unwrap(), vec![seg1, seg2]);
    }

    #[test]
    fn corrupt_mid_file_record_is_an_explicit_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("transcript.jsonl");

        let seg1 = make_segment(1);
        let mut raw = serde_json::to_string(&Record::Segment(seg1)).unwrap();
        raw.push('\n');
        raw.push_str("{\"kind\":\"segment\",\"id\":2,not json\n");
        raw.push_str(&serde_json::to_string(&Record::Segment(make_segment(3))).unwrap());
        raw.push('\n');
        fs::write(&path, raw).unwrap();

        let err = super::read_all(&path).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
        assert!(
            err.to_string().contains("line 2"),
            "error must name the corrupt line: {err}"
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
