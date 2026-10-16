//! System output sinks (PipeWire / PulseAudio) through the `pactl` CLI.
//!
//! On a desktop running a sound server, the server keeps the ALSA hardware of the
//! *active* card open exclusively. That card therefore never passes cpal's
//! "can this device be opened" probe, so the very device the user is listening to
//! is missing from the ALSA device list, while every idle card shows up. The layer
//! a desktop such as KDE actually switches between is the sink list, and `pactl`
//! is the CLI shipped with the client library - using it avoids a build-time
//! dependency on libpulse headers. When `pactl` is absent or no server answers,
//! callers fall back to the ALSA device list.
//!
//! The sink description is already composed and localized by the server
//! (`<card name> <profile description>`, e.g. `Example USB Audio 模拟立体声`), so it is
//! used as the display label as-is.

use std::process::Command;

const PACTL: &str = "pactl";

pub struct SystemSink {
    /// Sink name used by the sound server, e.g. `alsa_output.usb-...analog-stereo`
    pub name: String,
    /// Human readable, already localized description reported by the server
    pub description: String,
    /// Whether this is the sink the server currently routes new streams to
    pub is_default: bool,
}

/// Runs `pactl` with a C locale so the field labels stay parseable.
///
/// Returns `None` when the binary is missing, the server does not answer or the
/// command fails, which callers treat as "no sound server, use ALSA".
fn pactl(args: &[&str]) -> Option<String> {
    let output = Command::new(PACTL)
        .args(args)
        .env("LC_ALL", "C")
        .env("LANG", "C")
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8(output.stdout).ok()
}

/// Extracts `(name, description)` pairs from `pactl list sinks` output.
///
/// Every sink is a `Sink #<id>` header followed by tab-indented fields; property
/// list entries are indented twice and are skipped, so `Name:` and `Description:`
/// can only match a field of the current sink.
fn parse_sinks(text: &str) -> Vec<(String, String)> {
    let mut sinks = Vec::new();
    let mut name: Option<String> = None;
    let mut description: Option<String> = None;
    for line in text.lines() {
        let Some(field) = line.strip_prefix('\t') else {
            // A non-indented line is the next `Sink #<id>` header, so flush.
            if let (Some(name), Some(description)) = (name.take(), description.take()) {
                sinks.push((name, description));
            }
            continue;
        };
        if field.starts_with('\t') {
            continue;
        }
        if let Some(value) = field.strip_prefix("Name: ") {
            name = Some(value.trim().to_string());
        } else if let Some(value) = field.strip_prefix("Description: ") {
            description = Some(value.trim().to_string());
        }
    }
    if let (Some(name), Some(description)) = (name, description) {
        sinks.push((name, description));
    }
    sinks
}

/// Lists the sound server sinks, or `None` when there is no usable server.
pub fn list() -> Option<Vec<SystemSink>> {
    let sinks = parse_sinks(&pactl(&["list", "sinks"])?);
    if sinks.is_empty() {
        return None;
    }
    let default = pactl(&["get-default-sink"]).map(|text| text.trim().to_string());
    Some(
        sinks
            .into_iter()
            .map(|(name, description)| SystemSink {
                is_default: default.as_deref() == Some(name.as_str()),
                name,
                description,
            })
            .collect(),
    )
}

/// Routes new streams to `sink` by making it the server default.
///
/// Playback keeps using the default ALSA PCM, which the sound server maps to its
/// default sink, so the caller only has to reopen that stream afterwards.
pub fn set_default(sink: &str) -> Result<(), String> {
    let known = list().is_some_and(|sinks| sinks.iter().any(|candidate| candidate.name == sink));
    if !known {
        return Err(format!("unknown system output sink: {sink}"));
    }
    match pactl(&["set-default-sink", sink]) {
        Some(_) => Ok(()),
        None => Err(format!("could not set system output sink to {sink}")),
    }
}

#[cfg(test)]
mod tests {
    use super::parse_sinks;

    #[test]
    fn parses_name_and_description_per_sink() {
        let text = "Sink #53\n\tState: RUNNING\n\tName: alsa_output.usb-Example.analog-stereo\n\
                    \tDescription: Example USB Audio 模拟立体声\n\tDriver: PipeWire\n\tProperties:\n\
                    \t\talsa.card_name = \"Example USB Audio\"\n\t\tdevice.description = \"Example USB Audio\"\n\
                    Sink #57\n\tState: SUSPENDED\n\tName: alsa_output.usb-Example2.analog-stereo\n\
                    \tDescription: Example Surround Audio 模拟立体声\n";
        assert_eq!(
            parse_sinks(text),
            vec![
                ("alsa_output.usb-Example.analog-stereo".to_string(), "Example USB Audio 模拟立体声".to_string()),
                ("alsa_output.usb-Example2.analog-stereo".to_string(), "Example Surround Audio 模拟立体声".to_string()),
            ],
        );
    }

    #[test]
    fn ignores_property_lines_that_look_like_fields() {
        // `node.name` / `api.alsa.name` must not be mistaken for the sink name
        let text = "Sink #1\n\tName: real-sink\n\tDescription: Real sink\n\tProperties:\n\
                    \t\tnode.name = \"node-name\"\n\t\tapi.alsa.name = \"USB Audio\"\n";
        assert_eq!(parse_sinks(text), vec![("real-sink".to_string(), "Real sink".to_string())]);
    }

    #[test]
    fn skips_incomplete_blocks() {
        let text = "Sink #1\n\tName: only-name\nSink #2\n\tState: IDLE\n";
        assert!(parse_sinks(text).is_empty());
    }
}
