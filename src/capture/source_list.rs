use std::process::Command;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceKind {
    Input,
    Monitor,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioSource {
    pub name: String,
    pub description: String,
    pub kind: SourceKind,
}

/// Parse the stdout of `pactl list sources` into typed sources, separating
/// microphone inputs from output-sink monitors. Blocks begin with a line
/// starting `Source #`; within a block the tab-indented `Name:` and
/// `Description:` lines carry the node name and human label. A `Name:`
/// ending in `.monitor` marks a monitor; every other source is an input.
/// Blocks without a `Name:` are skipped; a missing `Description:` falls
/// back to the name.
pub fn parse_listing(output: &str) -> Vec<AudioSource> {
    let mut sources = Vec::new();
    let mut name: Option<String> = None;
    let mut description: Option<String> = None;

    for line in output.lines() {
        if line.starts_with("Source #") {
            push_source(&mut name, &mut description, &mut sources);
            continue;
        }
        let trimmed = line.trim_start();
        if let Some(rest) = trimmed.strip_prefix("Name:") {
            name = Some(rest.trim().to_string());
        } else if let Some(rest) = trimmed.strip_prefix("Description:") {
            description = Some(rest.trim().to_string());
        }
    }
    push_source(&mut name, &mut description, &mut sources);
    sources
}

fn push_source(
    name: &mut Option<String>,
    description: &mut Option<String>,
    sources: &mut Vec<AudioSource>,
) {
    let Some(name) = name.take() else {
        *description = None;
        return;
    };
    let kind = if name.ends_with(".monitor") {
        SourceKind::Monitor
    } else {
        SourceKind::Input
    };
    let description = description.take().unwrap_or_else(|| name.clone());
    sources.push(AudioSource {
        name,
        description,
        kind,
    });
}

/// Run `pactl list sources` and parse its stdout. A non-zero exit maps to
/// an `io::Error`.
pub fn enumerate() -> std::io::Result<Vec<AudioSource>> {
    let output = Command::new("pactl")
        .args(["list", "sources"])
        .output()
        .and_then(|output| {
            if !output.status.success() {
                return Err(std::io::Error::other(format!(
                    "pactl list sources exited with {status}",
                    status = output.status
                )));
            }
            Ok(output)
        })?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    Ok(parse_listing(&stdout))
}

/// Find the indices of the configured default sources within `sources`. A
/// mic default must match an `Input` source by name; a monitor default must
/// match a `Monitor` source by name. An absent default yields `None` for
/// that selection; `sources` itself is not filtered.
pub fn resolve_defaults(
    sources: &[AudioSource],
    mic_default: &str,
    monitor_default: &str,
) -> (Option<usize>, Option<usize>) {
    let mic = sources
        .iter()
        .position(|s| s.kind == SourceKind::Input && s.name == mic_default);
    let monitor = sources
        .iter()
        .position(|s| s.kind == SourceKind::Monitor && s.name == monitor_default);
    (mic, monitor)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Fixture built from the real `pactl list sources` shape: blocks
    // separated by blank lines, fields tab-indented, one input and one
    // monitor.
    const LISTING: &str = "\
Source #42
\tState: SUSPENDED
\tName: alsa_input.pci-0000_00_1f.3.analog-stereo
\tDescription: Built-in Audio Analog Stereo
\tDriver: PipeWire

Source #62
\tState: SUSPENDED
\tName: alsa_output.pci-0000_c4_00.6.analog-stereo.monitor
\tDescription: Monitor of Ryzen HD Audio Controller Analog Stereo
\tDriver: PipeWire

Source #99
\tState: SUSPENDED
\tName: alsa_input.usb-mic.mono
\tDescription: USB Microphone Mono
\tDriver: PipeWire
";

    #[test]
    fn parse_listing_separates_inputs_from_monitors_and_reads_fields() {
        let sources = parse_listing(LISTING);

        assert_eq!(sources.len(), 3);

        let inputs: Vec<&AudioSource> = sources
            .iter()
            .filter(|s| s.kind == SourceKind::Input)
            .collect();
        let monitors: Vec<&AudioSource> = sources
            .iter()
            .filter(|s| s.kind == SourceKind::Monitor)
            .collect();

        assert_eq!(inputs.len(), 2);
        assert_eq!(monitors.len(), 1);

        assert_eq!(inputs[0].name, "alsa_input.pci-0000_00_1f.3.analog-stereo");
        assert_eq!(inputs[0].description, "Built-in Audio Analog Stereo");
        assert_eq!(inputs[1].name, "alsa_input.usb-mic.mono");

        assert_eq!(
            monitors[0].name,
            "alsa_output.pci-0000_c4_00.6.analog-stereo.monitor"
        );
        assert_eq!(
            monitors[0].description,
            "Monitor of Ryzen HD Audio Controller Analog Stereo"
        );

        // A monitor is never reported as an input and vice versa.
        assert!(inputs.iter().all(|s| !s.name.ends_with(".monitor")));
        assert!(monitors.iter().all(|s| s.name.ends_with(".monitor")));
    }

    #[test]
    fn parse_listing_skips_block_without_name_and_falls_back_to_name_for_description() {
        let listing = "\
Source #1
\tState: SUSPENDED
\tDescription: Orphan block with no Name

Source #2
\tName: alsa_input.solo
";

        let sources = parse_listing(listing);
        assert_eq!(sources.len(), 1);
        assert_eq!(sources[0].name, "alsa_input.solo");
        assert_eq!(sources[0].description, "alsa_input.solo");
        assert_eq!(sources[0].kind, SourceKind::Input);
    }

    #[test]
    fn resolve_defaults_returns_none_for_absent_mic_and_some_for_present_monitor() {
        let sources = parse_listing(LISTING);

        let (mic, monitor) = resolve_defaults(
            &sources,
            "alsa_input.does-not-exist",
            "alsa_output.pci-0000_c4_00.6.analog-stereo.monitor",
        );

        assert_eq!(mic, None);
        // The monitor default sits at index 1 in the parsed listing.
        assert_eq!(monitor, Some(1));
        // The full slice is unchanged: resolve_defaults only reports
        // indices, it never filters.
        assert_eq!(sources.len(), 3);
    }

    #[test]
    fn resolve_defaults_returns_some_for_both_when_present() {
        let sources = parse_listing(LISTING);

        let (mic, monitor) = resolve_defaults(
            &sources,
            "alsa_input.usb-mic.mono",
            "alsa_output.pci-0000_c4_00.6.analog-stereo.monitor",
        );

        assert_eq!(mic, Some(2));
        assert_eq!(monitor, Some(1));
    }

    #[test]
    fn resolve_defaults_never_matches_a_monitor_name_as_mic_or_vice_versa() {
        let sources = parse_listing(LISTING);

        // A monitor name offered as the mic default must not resolve.
        let (mic, _) = resolve_defaults(
            &sources,
            "alsa_output.pci-0000_c4_00.6.analog-stereo.monitor",
            "alsa_output.pci-0000_c4_00.6.analog-stereo.monitor",
        );
        assert_eq!(mic, None);

        // An input name offered as the monitor default must not resolve.
        let (_, monitor) = resolve_defaults(
            &sources,
            "alsa_input.usb-mic.mono",
            "alsa_input.usb-mic.mono",
        );
        assert_eq!(monitor, None);
    }
}
