use std::process::{Child, Command, Stdio};

pub fn build_command(target_node: &str) -> Command {
    let mut cmd = Command::new("pw-record");
    cmd.args([
        "--target",
        target_node,
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
    build_command(target_node).spawn()
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

    #[test]
    fn build_command_targets_correct_node_with_expected_flags() {
        let cmd = build_command("alsa_input.usb-mic.mono");

        assert_eq!(cmd.get_program(), "pw-record");

        let args: Vec<&str> = cmd.get_args().map(|a| a.to_str().unwrap()).collect();

        let pair = |flag: &str, value: &str| {
            let idx = args
                .iter()
                .position(|a| *a == flag)
                .unwrap_or_else(|| panic!("missing flag {flag}"));
            assert_eq!(args[idx + 1], value, "wrong value for {flag}");
        };

        pair("--target", "alsa_input.usb-mic.mono");
        pair("--rate", "16000");
        pair("--channels", "1");
        pair("--format", "s16");
        assert!(args.contains(&"--raw"));
        assert_eq!(args.last(), Some(&"-"), "must write raw PCM to stdout");
    }
}
