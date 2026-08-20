pub mod capture;
pub mod confidence;
pub mod config;
pub mod domain;
pub mod history;
mod http;
pub mod language;
pub mod log;
pub mod pipeline;
pub mod recording;
mod scheduler;
pub mod segmenter;
pub mod store;
pub mod transcriber;
pub mod transcript;
pub mod translator;

pub use domain::{Segment, SpeakerTag};
