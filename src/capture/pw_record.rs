use std::process::{Child, Command, Stdio};

/// PipeWire has no node named `*.monitor` — that name exists only in the
/// PulseAudio compat layer that `pactl list sources` reports. Passing it to
/// `--target` matches nothing, and pw-record then silently falls back to the
/// default source, i.e. the microphone, so both Streams capture the same
/// audio. A sink's output is reached by targeting the sink node itself and
/// asking for its monitor ports.
pub fn build_command(target_node: &str) -> Command {
    let mut cmd = Command::new("pw-record");
    match target_node.strip_suffix(".monitor") {
        Some(sink) => cmd.args(["-P", "{ stream.capture.sink=true }", "--target", sink]),
        None => cmd.args(["--target", target_node]),
    };
    cmd.args([
        "--rate",
        "16000",
        "--channels",
        "1",
        "--format",
        "s16",
        "--raw",
        "-",
    ]);
    cmd.stdout(Stdio::piped());
    cmd
}

pub fn spawn(target_node: &str) -> std::io::Result<Child> {
    let mut command = build_command(target_node);
    crate::debug!(
        "capture: pw-record {}",
        command
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join(" ")
    );
    command.spawn()
}

/// Reaps the process via SIGKILL-equivalent `Child::kill`. `pw-record` gets no
/// chance to flush or exit gracefully, but its own subprocess isolation is the
/// safety net here, not shutdown semantics; we only need it gone and reaped.
pub fn stop(child: &mut Child) -> std::io::Result<()> {
    child.kill()?;
    child.wait()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args_of(cmd: &Command) -> Vec<String> {
        cmd.get_args()
            .map(|a| a.to_str().unwrap().to_string())
            .collect()
    }

    fn value_after(args: &[String], flag: &str) -> String {
        let idx = args
            .iter()
            .position(|a| a == flag)
            .unwrap_or_else(|| panic!("missing flag {flag}"));
        args[idx + 1].clone()
    }

    #[test]
    fn build_command_targets_correct_node_with_expected_flags() {
        let cmd = build_command("alsa_input.usb-mic.mono");

        assert_eq!(cmd.get_program(), "pw-record");

        let args = args_of(&cmd);

        assert_eq!(value_after(&args, "--target"), "alsa_input.usb-mic.mono");
        assert_eq!(value_after(&args, "--rate"), "16000");
        assert_eq!(value_after(&args, "--channels"), "1");
        assert_eq!(value_after(&args, "--format"), "s16");
        assert!(args.iter().any(|a| a == "--raw"));
        assert_eq!(args.last().map(String::as_str), Some("-"));
        assert!(
            !args.iter().any(|a| a == "-P"),
            "an input source needs no sink-capture property"
        );
    }

    #[test]
    fn monitor_target_captures_the_sink_node_not_the_pulse_monitor_name() {
        let cmd = build_command("alsa_output.pci-0000_c4_00.6.analog-stereo.monitor");

        let args = args_of(&cmd);

        // The `.monitor` suffix matches no PipeWire node; targeting it makes
        // pw-record fall back to the default source.
        assert_eq!(
            value_after(&args, "--target"),
            "alsa_output.pci-0000_c4_00.6.analog-stereo"
        );
        assert_eq!(value_after(&args, "-P"), "{ stream.capture.sink=true }");
        assert_eq!(value_after(&args, "--rate"), "16000");
        assert_eq!(args.last().map(String::as_str), Some("-"));
    }
}
