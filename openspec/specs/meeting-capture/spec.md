# meeting-capture Specification

## Purpose
TBD - created by archiving change add-live-transcription-spine. Update Purpose after archive.
## Requirements
### Requirement: Dual Stream capture

A Recording SHALL capture two Streams for its full duration: the Mic Stream from the configured microphone source, tagged `me`, and the Monitor Stream from the configured output sink monitor, tagged `them`. Each Stream SHALL be captured as 16 kHz mono audio and persisted to its own WAV file inside the Recording's directory.

#### Scenario: Recording starts with both sources available

- **WHEN** a Recording is started and both configured sources exist in the audio graph
- **THEN** both Streams begin capturing and each writes to its own WAV file in the Recording's directory

#### Scenario: Recording stops

- **WHEN** the user stops the Recording
- **THEN** both captures terminate and both WAV files are finalized and playable

### Requirement: Segment boundaries from voice activity

The system SHALL cut each Stream into Segments using voice activity detection, independently per Stream. A Segment SHALL open when speech is detected, close after a configured silence hold, and be force-closed at a configured duration cap. Audio containing no detected speech SHALL NOT be submitted for transcription.

#### Scenario: Speech followed by silence

- **WHEN** speech is detected on a Stream and is followed by silence exceeding the configured hold
- **THEN** a Segment is closed whose start and end timestamps bracket that speech

#### Scenario: Speech exceeding the duration cap

- **WHEN** speech continues past the configured duration cap without a qualifying silence
- **THEN** the Segment is force-closed at the cap and a new Segment opens immediately, losing no audio between them

#### Scenario: Extended silence on a Stream

- **WHEN** a Stream carries only silence
- **THEN** no Segment is produced and no transcription request is made for that Stream

### Requirement: Segment transcription

Each closed Segment SHALL be submitted to the configured whisper server as a single request, requesting a response format that carries per-word confidence. When a source language is configured for a Stream, that language SHALL be sent with the request. Requests SHALL be submitted as Segments close, without an application-side queue.

#### Scenario: Segment closes

- **WHEN** a Segment closes on either Stream
- **THEN** a transcription request for that Segment is submitted immediately

#### Scenario: Source language configured

- **WHEN** a Stream has a source language configured
- **THEN** the transcription request for its Segments carries that language rather than relying on auto-detection

### Requirement: Low-confidence Segment discard

A transcribed Segment whose mean word confidence falls below the configured floor SHALL be discarded: it SHALL NOT appear in the transcript and SHALL NOT be persisted.

#### Scenario: Non-speech audio transcribed into text

- **WHEN** a Segment transcribes to text whose mean word confidence is below the configured floor
- **THEN** the Segment is discarded and never appears in the transcript

#### Scenario: Confident transcription

- **WHEN** a Segment transcribes with mean word confidence at or above the floor
- **THEN** the Segment enters the transcript unchanged

### Requirement: Transcript ordering and speaker tagging

Segments from both Streams SHALL form one transcript ordered by each Segment's start timestamp relative to Recording start. Order SHALL NOT depend on the order transcription responses arrive in. Every Segment SHALL carry the Speaker tag of the Stream it came from.

#### Scenario: Responses arrive out of order

- **WHEN** a Segment that started earlier is transcribed after a Segment that started later
- **THEN** the earlier Segment occupies the earlier position in the transcript

#### Scenario: Both sides speak

- **WHEN** Segments are produced on both the Mic Stream and the Monitor Stream
- **THEN** the transcript interleaves them by start timestamp, each tagged `me` or `them` according to its Stream

### Requirement: Crash-safe transcript persistence

Each finalized Segment SHALL be appended to the Recording's transcript file as it finalizes, and flushed. Every Segment SHALL carry an identifier unique within its Recording, so that later records can refer to it. Reading a transcript file whose final line is incomplete SHALL yield every complete line without error.

#### Scenario: Process killed mid-Recording

- **WHEN** the application is killed during a Recording
- **THEN** the transcript file contains every Segment finalized before the kill and remains readable

#### Scenario: Transcript file with a truncated final line

- **WHEN** a transcript file is read whose last line was partially written
- **THEN** the incomplete line is skipped and all preceding Segments are returned

### Requirement: Streams bind to the sources chosen for the Recording

A Recording's Mic Stream and Monitor Stream SHALL bind to the sources selected for that Recording, rather than directly to the names in configuration. Configured names SHALL serve only as the defaults offered before a Recording starts.

#### Scenario: Recording started with a non-default source

- **WHEN** a Recording is started with a microphone other than the configured default
- **THEN** its Mic Stream captures from the selected source for the whole Recording

### Requirement: Recording title

Every Recording SHALL carry a title, persisted with the Recording.

#### Scenario: Recording ends

- **WHEN** a Recording is stopped
- **THEN** its title and the names of the sources it captured are readable from its directory

### Requirement: Recording finalization on stop

Stopping a Recording SHALL write its metadata to its directory: title, start time, duration, the names of the sources captured, and the target language used for translation.

#### Scenario: Recording is stopped

- **WHEN** the user stops a Recording
- **THEN** its metadata is written to its directory and the Recording is listable in history

#### Scenario: Application is killed before stopping

- **WHEN** the application is killed during a Recording, so that no metadata is written
- **THEN** the Recording's transcript and audio remain on disk and the incomplete directory does not prevent other Recordings from being listed

