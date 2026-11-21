//! 输出设备枚举、命名与默认设备变化监听

use super::*;

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioOutputDevice {
    /// Device identifier sent back when switching; opaque to the frontend
    pub name: String,
    /// Fallback display name: the alsa-lib description, or the PCM name
    pub label: String,
    /// `sink` for a sound server output, `device` for a cpal/ALSA device
    pub kind: String,
    /// Card display name matching KDE (e.g. `Example USB Audio`); None off Linux
    pub card: Option<String>,
    /// Role key (`analog_stereo`, `hdmi`, ...) resolved to text by the frontend
    pub role: Option<String>,
    /// Role detail (surround layout, HDMI index)
    pub detail: Option<String>,
    pub is_default: bool,
}

/// Splits a PCM name such as `front:CARD=USB0,DEV=0` into (prefix, card id, device)
#[cfg(target_os = "linux")]
pub(super) fn split_pcm_name(name: &str) -> (&str, Option<&str>, Option<&str>) {
    let mut parts = name.splitn(2, ':');
    let prefix = parts.next().unwrap_or(name);
    let mut card = None;
    let mut device = None;
    if let Some(rest) = parts.next() {
        for field in rest.split(',') {
            if let Some(value) = field.strip_prefix("CARD=") {
                card = Some(value);
            } else if let Some(value) = field.strip_prefix("DEV=") {
                device = Some(value);
            }
        }
    }
    (prefix, card, device)
}

/// Maps a PCM prefix to (role key, role detail)
///
/// The role drives the label shown in the UI (`Example USB Audio 模拟立体声`), matching how
/// KDE/Plasma names a card together with its profile.
#[cfg(target_os = "linux")]
pub(super) fn pcm_role(
    prefix: &str,
    device: Option<&str>,
) -> Option<(&'static str, Option<String>)> {
    let surround = |layout: &str| Some(("surround", Some(layout.to_string())));
    let numbered = |role: &'static str| Some((role, device.map(str::to_string)));
    match prefix {
        "sysdefault" => Some(("system_default", None)),
        "front" | "analog" => Some(("analog_stereo", None)),
        "surround21" => surround("2.1"),
        "surround40" => surround("4.0"),
        "surround41" => surround("4.1"),
        "surround50" => surround("5.0"),
        "surround51" => surround("5.1"),
        "surround71" => surround("7.1"),
        "hdmi" => numbered("hdmi"),
        "iec958" => numbered("spdif"),
        "hw" => Some(("hardware", None)),
        "plughw" => Some(("hardware_converted", None)),
        "dmix" => Some(("software_mix", None)),
        "usbstream" => Some(("usb_stream", None)),
        "lavrate" | "samplerate" | "speexrate" | "speex" | "upmix" | "vdownmix" => {
            Some(("converter", None))
        }
        _ => None,
    }
}

/// Card id from `/proc/asound/cards` to the name the driver reports, e.g.
/// ` 1 [USB0          ]: USB-Audio - Example USB Audio` → `USB0` → `Example USB Audio`
#[cfg(target_os = "linux")]
fn alsa_card_names() -> std::collections::HashMap<String, String> {
    std::fs::read_to_string("/proc/asound/cards")
        .map(|text| parse_alsa_cards(&text))
        .unwrap_or_default()
}

#[cfg(not(target_os = "linux"))]
fn alsa_card_names() -> std::collections::HashMap<String, String> {
    std::collections::HashMap::new()
}

#[cfg(target_os = "linux")]
pub(super) fn parse_alsa_cards(text: &str) -> std::collections::HashMap<String, String> {
    let mut cards = std::collections::HashMap::new();
    for line in text.lines() {
        // Only the "index [id]: driver - name" line counts; indented continuation
        // lines (the device path and irq) are skipped
        let Some((head, tail)) = line.split_once("]: ") else {
            continue;
        };
        let Some(id) = head.split_once('[').map(|(_, id)| id.trim()) else {
            continue;
        };
        let Some(name) = tail.split_once(" - ").map(|(_, name)| name.trim()) else {
            continue;
        };
        if !id.is_empty() && !name.is_empty() {
            cards.insert(id.to_string(), name.to_string());
        }
    }
    cards
}

/// The first line of an alsa-lib description is "card name, chip name"; this is the
/// fallback when `/proc/asound/cards` has no entry
#[cfg(target_os = "linux")]
pub(super) fn card_name_from_hint(description: &str) -> Option<String> {
    let first = description.lines().next()?.trim();
    let name = first.split(',').next().unwrap_or(first).trim();
    (!name.is_empty()).then(|| name.to_string())
}

/// Linux: reads alsa-lib PCM hints for (name, human readable description) without ever
/// opening a device
///
/// Returns an empty list when the hints are unavailable; callers then fall back to the
/// cpal enumeration.
#[cfg(target_os = "linux")]
fn alsa_pcm_hints() -> Vec<(String, Option<String>)> {
    use alsa::device_name::HintIter;
    use alsa::Direction;
    let Ok(hints) = HintIter::new_str(None, "pcm") else {
        return Vec::new();
    };
    hints
        .filter_map(|hint| {
            let alsa::device_name::Hint {
                name,
                desc,
                direction,
            } = hint;
            // A PCM whose IOID is Input can only record (alsa-lib does not filter hints
            // by direction)
            if direction == Some(Direction::Capture) {
                return None;
            }
            name.map(|name| (name, desc))
        })
        .collect()
}

/// On Linux cpal only knows the PCM name (`sysdefault:CARD=ONBOARD`, `dmix:CARD=ONBOARD,DEV=0`),
/// while the alsa-lib hint carries a human readable description (`Example Onboard Audio, ALC000
/// Analog`)
#[cfg(target_os = "linux")]
fn device_labels() -> std::collections::HashMap<String, String> {
    let mut labels = std::collections::HashMap::new();
    for (name, description) in alsa_pcm_hints() {
        let Some(description) = description else {
            continue;
        };
        // The description may span lines (device name plus purpose); squash it into one
        // before handing it to the UI
        let label = description.split_whitespace().collect::<Vec<_>>().join(" ");
        if !label.is_empty() {
            labels.entry(name).or_insert(label);
        }
    }
    labels
}

#[cfg(not(target_os = "linux"))]
fn device_labels() -> std::collections::HashMap<String, String> {
    std::collections::HashMap::new()
}

pub struct OutputDeviceChangeRequest {
    pub(super) receiver: mpsc::Receiver<Result<(), String>>,
    pub(super) cancel: Arc<AtomicBool>,
    pub(super) completed: bool,
}

impl OutputDeviceChangeRequest {
    pub fn wait(mut self) -> AppResult<()> {
        let result = self.receiver.recv_timeout(COMMAND_TIMEOUT);
        self.completed = result.is_ok();
        result
            .map_err(|error| AppError::Audio(error.to_string()))?
            .map_err(AppError::Audio)
    }
}

impl Drop for OutputDeviceChangeRequest {
    fn drop(&mut self) {
        if !self.completed {
            self.cancel.store(true, Ordering::Release);
        }
    }
}

/// 能输出的设备，按默认混音格式判断
///
/// 不用 cpal 的 `output_devices()`：它对每个设备试探几十种格式，cpal 0.15 在 WASAPI 上
/// 不释放 `IsFormatSupported` 返回的近似格式，每次枚举泄漏约 23 KB（7 个设备实测）。
/// 设备监视每 2 秒枚举一次，一小时就是三十多 MB。
pub(super) fn render_devices(
    host: &cpal::Host,
) -> Result<impl Iterator<Item = Device>, cpal::DevicesError> {
    Ok(host
        .devices()?
        .filter(|device| device.default_output_config().is_ok()))
}

pub fn list_audio_output_devices() -> AppResult<Vec<AudioOutputDevice>> {
    crate::quiet::silence_library_diagnostics();
    // Linux with a sound server: list its sinks instead of ALSA PCMs, because the
    // server holds the active card exclusively and that card is exactly the one
    // missing from the cpal list (see `audio::system_sinks`).
    #[cfg(target_os = "linux")]
    if let Some(sinks) = crate::system_sinks::list() {
        return Ok(sinks
            .into_iter()
            .map(|sink| AudioOutputDevice {
                is_default: sink.is_default,
                name: sink.name,
                label: sink.description,
                kind: "sink".to_string(),
                card: None,
                role: None,
                detail: None,
            })
            .collect());
    }
    let host = cpal::default_host();
    let default_name = host
        .default_output_device()
        .and_then(|device| device.name().ok());
    let devices = render_devices(&host).map_err(|error| AppError::Audio(error.to_string()))?;
    let labels = device_labels();
    let cards = alsa_card_names();
    let mut result = Vec::new();
    for device in devices {
        let Ok(name) = device.name() else { continue };
        if result
            .iter()
            .any(|item: &AudioOutputDevice| item.name == name)
        {
            continue;
        }
        let description = labels.get(&name);
        let label = description.cloned().unwrap_or_else(|| name.clone());
        let (card, role, detail) = describe_output_device(&name, description, &cards);
        result.push(AudioOutputDevice {
            is_default: default_name.as_deref() == Some(name.as_str()),
            name,
            label,
            kind: "device".to_string(),
            card,
            role,
            detail,
        });
    }
    Ok(result)
}

/// Splits out "card + role" so the UI can render KDE-style names such as
/// `Example USB Audio 模拟立体声`
///
/// The card name comes from `/proc/asound/cards` first (identical to what KDE shows),
/// then from the first line of the alsa-lib description. When neither is available the
/// card stays unset instead of being guessed, and the description is shown as-is.
#[cfg(target_os = "linux")]
pub(super) fn describe_output_device(
    name: &str,
    description: Option<&String>,
    cards: &std::collections::HashMap<String, String>,
) -> (Option<String>, Option<String>, Option<String>) {
    let (prefix, card_id, device) = split_pcm_name(name);
    let Some((role, detail)) = pcm_role(prefix, device) else {
        return (None, None, None);
    };
    let card = card_id.and_then(|id| {
        cards
            .get(id)
            .cloned()
            .or_else(|| description.and_then(|text| card_name_from_hint(text)))
    });
    (card, Some(role.to_string()), detail)
}

#[cfg(not(target_os = "linux"))]
pub(super) fn describe_output_device(
    _name: &str,
    _description: Option<&String>,
    _cards: &std::collections::HashMap<String, String>,
) -> (Option<String>, Option<String>, Option<String>) {
    (None, None, None)
}

pub(super) struct OutputDeviceState {
    pub(super) preferred_name: Option<String>,
    pub(super) profile: Option<OutputDeviceProfile>,
    /// Sound-server sink label to keep as the DSP profile key when the ALSA profile is default.
    pub(super) dsp_output_name: Option<String>,
}

/// `prepare_session` always selects the ALSA profile name; restore the sink label
/// afterwards so per-output DSP profiles survive seeks, rebuilds, and device loss.
pub(super) fn restore_dsp_output_identity(effects: &EffectsControl, output: &OutputDeviceState) {
    let name = output
        .dsp_output_name
        .as_deref()
        .or_else(|| output.profile.as_ref().map(|profile| profile.name.as_str()));
    if let Some(name) = name {
        effects.dsp.select_output(name);
    }
}

pub(super) fn effective_output_name<'a>(
    preferred: Option<&'a str>,
    available: &[&str],
    default: Option<&'a str>,
) -> Option<&'a str> {
    preferred
        .filter(|name| available.contains(name))
        .or(default)
}

/// 设备列表更新后要切到的输出设备；已经在用目标设备时返回 None
pub(super) fn output_device_to_switch(
    preferred: Option<&str>,
    available: &[String],
    default: Option<&str>,
    current: Option<&str>,
) -> Option<String> {
    let names: Vec<&str> = available.iter().map(String::as_str).collect();
    let target = effective_output_name(preferred, &names, default)?;
    (current != Some(target)).then(|| target.to_string())
}

/// Snapshot for the device watcher: ALSA device names plus the ALSA default.
///
/// Deliberately not `list_audio_output_devices()`: that one may list sound server
/// sinks through `pactl`, which must not be spawned on every poll. The engine also
/// matches its preferred ALSA device against these names, so sinks do not belong here.
fn poll_alsa_output_devices() -> AppResult<(Vec<String>, Option<String>)> {
    crate::quiet::silence_library_diagnostics();
    let host = cpal::default_host();
    let default = host
        .default_output_device()
        .and_then(|device| device.name().ok());
    let available = render_devices(&host)
        .map_err(|error| AppError::Audio(error.to_string()))?
        .filter_map(|device| device.name().ok())
        .collect();
    Ok((available, default))
}

/// 在独立线程里定期枚举输出设备，把结果交给控制线程
///
/// Windows 上一次枚举要 250–300 ms。放在控制线程里时，这段时间到达的
/// 播放、seek、暂停命令都得排队等它。控制线程断开后，监视线程随之退出
pub(super) fn spawn_output_device_watch(
    commands: mpsc::Sender<AudioCmd>,
) -> Option<mpsc::Sender<bool>> {
    let (activity_tx, activity_rx) = mpsc::channel();
    let spawned = thread::Builder::new()
        .name("audio-device-watch".into())
        .spawn(move || {
            let mut last_error: Option<String> = None;
            let mut active = false;
            while wait_for_output_device_poll(&activity_rx, &mut active) {
                let (available, default) = match poll_alsa_output_devices() {
                    Ok(snapshot) => {
                        if last_error.take().is_some() {
                            log::info!(target: "cpal-output", "output device enumeration recovered");
                        }
                        snapshot
                    }
                    Err(error) => {
                        let message = error.to_string();
                        if last_error.as_deref() != Some(message.as_str()) {
                            log::warn!(target: "cpal-output", "output device enumeration failed: {message}");
                        }
                        last_error = Some(message);
                        continue;
                    }
                };
                if commands
                    .send(AudioCmd::OutputDevicesListed { available, default })
                    .is_err()
                {
                    break;
                }
            }
        });
    match spawned {
        Ok(_) => Some(activity_tx),
        Err(error) => {
            log::error!(target: "cpal-output", "could not start the output device watcher: {error}");
            None
        }
    }
}

pub(super) fn wait_for_output_device_poll(
    receiver: &mpsc::Receiver<bool>,
    active: &mut bool,
) -> bool {
    loop {
        let interval = if *active {
            OUTPUT_DEVICE_POLL_INTERVAL
        } else {
            OUTPUT_DEVICE_IDLE_POLL_INTERVAL
        };
        match receiver.recv_timeout(interval) {
            Ok(next_active) => {
                let started_playing = next_active && !*active;
                *active = next_active;
                // 状态变化直接唤醒，不需要另设空闲 tick；恢复播放时立即检查设备
                if started_playing {
                    return true;
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => return true,
            Err(mpsc::RecvTimeoutError::Disconnected) => return false,
        }
    }
}

pub(super) fn matches_output_session(
    current_generation: u64,
    current_cancel: &Arc<AtomicBool>,
    reported_generation: u64,
    reported_cancel: &Arc<AtomicBool>,
) -> bool {
    current_generation == reported_generation && Arc::ptr_eq(current_cancel, reported_cancel)
}
