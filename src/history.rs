use crate::domain::Segment;
use crate::store;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordingEntry {
    pub dir: PathBuf,
    pub title: String,
    pub start_time: u64,
    pub duration_secs: Option<u64>,
    /// Target language this Recording was translated into, which is
    /// what decides whether a Segment of it was same-language. Absent
    /// on Recordings written before it was recorded.
    pub target_language: Option<String>,
    pub line_count: usize,
}

// Permissive deserialize: pull only the fields the listing needs and
// tolerate an absent duration_secs (recordings that never reached
// stop) or an absent start_time (older on-disk shape). A directory
// whose meta.json can't be parsed into title+start_time is skipped by
// the caller, not fatal to the listing.
#[derive(serde::Deserialize)]
struct MetaSummary {
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    start_time: Option<u64>,
    #[serde(default)]
    duration_secs: Option<u64>,
    #[serde(default)]
    target_language: Option<String>,
}

pub fn list_recordings(store_root: &Path) -> io::Result<Vec<RecordingEntry>> {
    let mut entries: Vec<RecordingEntry> = Vec::new();

    for entry in fs::read_dir(store_root)? {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }

        let meta_path = path.join("meta.json");
        let meta_bytes = match fs::read(&meta_path) {
            Ok(b) => b,
            Err(_) => continue,
        };
        let summary: MetaSummary = match serde_json::from_slice(&meta_bytes) {
            Ok(s) => s,
            Err(_) => continue,
        };
        let (Some(title), Some(start_time)) = (summary.title, summary.start_time) else {
            continue;
        };

        let line_count = store::read_all(&path.join("segments.jsonl"))
            .map(|segments| segments.len())
            .unwrap_or(0);

        entries.push(RecordingEntry {
            dir: path,
            title,
            start_time,
            duration_secs: summary.duration_secs,
            target_language: summary.target_language,
            line_count,
        });
    }

    entries.sort_by(|a, b| b.start_time.cmp(&a.start_time));
    Ok(entries)
}

pub fn read_transcript(dir: &Path) -> io::Result<Vec<Segment>> {
    store::read_all(&dir.join("segments.jsonl"))
}

pub fn delete_recording(dir: &Path) -> io::Result<()> {
    fs::remove_dir_all(dir)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::SpeakerTag;
    use crate::store::{Record, Store};
    use std::path::PathBuf;

    fn write_meta(dir: &Path, title: &str, start_time: u64, duration_secs: Option<u64>) {
        let mut json = format!(
            r#"{{"title":"{title}","mic_source":"mic","monitor_source":"mon","start_time":{start_time},"target_language":"en"}}"#
        );
        if let Some(d) = duration_secs {
            json = json.trim_end_matches('}').to_string() + &format!(",\"duration_secs\":{d}}}");
        }
        fs::write(dir.join("meta.json"), json).unwrap();
    }

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
    fn list_recordings_returns_newest_first_and_skips_dirs_without_meta() {
        let root = tempfile::tempdir().unwrap();

        let older = root.path().join("100");
        let newer = root.path().join("200");
        let no_meta = root.path().join("999");
        fs::create_dir_all(&older).unwrap();
        fs::create_dir_all(&newer).unwrap();
        fs::create_dir_all(&no_meta).unwrap();

        write_meta(&older, "Older", 100, Some(60));
        write_meta(&newer, "Newer", 200, Some(120));

        let mut store = Store::open(&older.join("segments.jsonl")).unwrap();
        store.append(&make_segment(1)).unwrap();
        store.append(&make_segment(2)).unwrap();
        drop(store);

        let mut store = Store::open(&newer.join("segments.jsonl")).unwrap();
        store.append(&make_segment(1)).unwrap();
        drop(store);

        let entries = list_recordings(root.path()).unwrap();
        assert_eq!(entries.len(), 2, "dir without meta.json must be skipped");
        assert_eq!(entries[0].title, "Newer");
        assert_eq!(entries[0].start_time, 200);
        assert_eq!(entries[0].duration_secs, Some(120));
        assert_eq!(entries[0].line_count, 1);
        assert_eq!(entries[1].title, "Older");
        assert_eq!(entries[1].start_time, 100);
        assert_eq!(entries[1].duration_secs, Some(60));
        assert_eq!(entries[1].line_count, 2);
    }

    #[test]
    fn list_recordings_tolerates_missing_duration_secs() {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("100");
        fs::create_dir_all(&dir).unwrap();
        write_meta(&dir, "In Progress", 100, None);

        let entries = list_recordings(root.path()).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].duration_secs, None);
        assert_eq!(entries[0].line_count, 0);
    }

    #[test]
    fn list_recordings_skips_dir_with_unparseable_meta() {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("100");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("meta.json"), "not json").unwrap();

        let entries = list_recordings(root.path()).unwrap();
        assert!(entries.is_empty());
    }

    #[test]
    fn read_transcript_merges_translation_records_and_preserves_untranslated_segments() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("segments.jsonl");

        let s1 = make_segment(1);
        let s2 = make_segment(2);

        let mut store = Store::open(&path).unwrap();
        store.append(&s1).unwrap();
        store
            .append_record(&Record::Translation {
                segment_id: 1,
                text: "hola".to_string(),
                degraded: false,
            })
            .unwrap();
        store.append(&s2).unwrap();

        let read_back = read_transcript(dir.path()).unwrap();
        assert_eq!(read_back.len(), 2);
        assert_eq!(read_back[0].id, 1);
        assert_eq!(read_back[0].translation.as_deref(), Some("hola"));
        assert!(!read_back[0].degraded);
        assert_eq!(read_back[1].id, 2);
        assert_eq!(read_back[1].translation, None);
    }

    #[test]
    fn read_transcript_preserves_degraded_flag_from_translation_record() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("segments.jsonl");

        let s1 = make_segment(1);

        let mut store = Store::open(&path).unwrap();
        store.append(&s1).unwrap();
        store
            .append_record(&Record::Translation {
                segment_id: 1,
                text: "hola".to_string(),
                degraded: true,
            })
            .unwrap();

        let read_back = read_transcript(dir.path()).unwrap();
        assert_eq!(read_back.len(), 1);
        assert_eq!(read_back[0].translation.as_deref(), Some("hola"));
        assert!(read_back[0].degraded);
    }

    #[test]
    fn delete_recording_removes_directory_and_audio() {
        let root = tempfile::tempdir().unwrap();
        let dir: PathBuf = root.path().join("100");
        fs::create_dir_all(&dir).unwrap();
        write_meta(&dir, "Doomed", 100, Some(30));

        let mut store = Store::open(&dir.join("segments.jsonl")).unwrap();
        store.append(&make_segment(1)).unwrap();
        drop(store);
        fs::write(dir.join("mic.wav"), b"RIFF....").unwrap();

        assert!(dir.exists());
        assert!(dir.join("mic.wav").exists());

        delete_recording(&dir).unwrap();

        assert!(!dir.exists());
        let entries = list_recordings(root.path()).unwrap();
        assert!(entries.is_empty());
    }
}
