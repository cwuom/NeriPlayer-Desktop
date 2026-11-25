//! Windows WASAPI 独占模式输出（cpal 0.15 只实现了共享模式）
//!
//! 所有 COM 对象都在专用渲染线程里创建和使用，避免跨线程传递非 Send 的接口；
//! 外部只通过命令通道控制播放/暂停/停止，初始化结果经同步通道回报。
//! 采用事件驱动：设备每消耗完一个周期就触发事件，线程整块填满一个周期的缓冲。

use std::ffi::OsString;
use std::os::windows::ffi::OsStringExt;
use std::sync::mpsc;
use std::thread::JoinHandle;
use std::time::Duration;

use windows::core::PCSTR;
use windows::Win32::Devices::Properties::DEVPKEY_Device_FriendlyName;
use windows::Win32::Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0};
use windows::Win32::Media::Audio::{
    eConsole, eRender, IAudioClient, IAudioRenderClient, IMMDevice, IMMDeviceEnumerator,
    MMDeviceEnumerator, AUDCLNT_BUFFERFLAGS_SILENT, AUDCLNT_E_BUFFER_SIZE_NOT_ALIGNED,
    AUDCLNT_E_DEVICE_INVALIDATED, AUDCLNT_SHAREMODE_EXCLUSIVE, AUDCLNT_STREAMFLAGS_EVENTCALLBACK,
    DEVICE_STATE_ACTIVE, WAVEFORMATEX, WAVEFORMATEXTENSIBLE, WAVEFORMATEXTENSIBLE_0,
};
use windows::Win32::Media::KernelStreaming::{KSDATAFORMAT_SUBTYPE_PCM, WAVE_FORMAT_EXTENSIBLE};
use windows::Win32::Media::Multimedia::KSDATAFORMAT_SUBTYPE_IEEE_FLOAT;
use windows::Win32::System::Com::StructuredStorage::PropVariantClear;
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_ALL, COINIT_MULTITHREADED, STGM_READ,
};
use windows::Win32::System::Threading::{CreateEventA, WaitForSingleObject};
use windows::Win32::System::Variant::VT_LPWSTR;

use super::exclusive_format::{
    channel_mask, frames_to_hns, ExclusiveFormat, ExclusiveSampleKind, EXCLUSIVE_FORMAT_CANDIDATES,
};

/// 渲染回调拿到的设备缓冲，按协商出的样本类型区分
pub enum ExclusiveBuffer<'a> {
    F32(&'a mut [f32]),
    I32(&'a mut [i32]),
    I16(&'a mut [i16]),
}

enum Command {
    Play,
    Pause,
    Stop,
}

/// 事件等待超时：超时只是回头检查命令，不代表出错
const EVENT_WAIT_MS: u32 = 200;
const INIT_TIMEOUT: Duration = Duration::from_secs(5);

pub struct ExclusiveStream {
    commands: mpsc::Sender<Command>,
    worker: Option<JoinHandle<()>>,
    pub format: ExclusiveFormat,
    pub buffer_frames: u32,
}

impl ExclusiveStream {
    /// endpoint 为 None 时用系统默认输出；名字与 cpal 的设备名（友好名称）一致
    pub fn open<R, E>(
        endpoint: Option<String>,
        sample_rate: u32,
        channels: u16,
        render: R,
        on_error: E,
    ) -> Result<Self, String>
    where
        R: FnMut(ExclusiveBuffer<'_>) + Send + 'static,
        E: FnMut(String) + Send + 'static,
    {
        let (commands, command_rx) = mpsc::channel();
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let worker = std::thread::Builder::new()
            .name("wasapi-exclusive".into())
            .spawn(move || {
                run(
                    endpoint,
                    sample_rate,
                    channels,
                    render,
                    on_error,
                    command_rx,
                    ready_tx,
                )
            })
            .map_err(|error| format!("Could not start exclusive output thread: {error}"))?;
        match ready_rx.recv_timeout(INIT_TIMEOUT) {
            Ok(Ok((format, buffer_frames))) => Ok(Self {
                commands,
                worker: Some(worker),
                format,
                buffer_frames,
            }),
            Ok(Err(error)) => {
                let _ = worker.join();
                Err(error)
            }
            Err(_) => {
                // 初始化卡住（驱动无响应）：让线程自行收尾，不在这里阻塞 join
                let _ = commands.send(Command::Stop);
                Err("Timed out opening exclusive audio output".into())
            }
        }
    }

    pub fn play(&self) -> Result<(), String> {
        self.commands
            .send(Command::Play)
            .map_err(|_| "Exclusive audio output thread has stopped".to_string())
    }

    pub fn pause(&self) -> Result<(), String> {
        self.commands
            .send(Command::Pause)
            .map_err(|_| "Exclusive audio output thread has stopped".to_string())
    }
}

impl Drop for ExclusiveStream {
    fn drop(&mut self) {
        let _ = self.commands.send(Command::Stop);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

struct EventHandle(HANDLE);

impl Drop for EventHandle {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

struct ComGuard;

impl Drop for ComGuard {
    fn drop(&mut self) {
        unsafe { CoUninitialize() };
    }
}

struct Opened {
    client: IAudioClient,
    render_client: IAudioRenderClient,
    event: EventHandle,
    format: ExclusiveFormat,
    buffer_frames: u32,
}

#[allow(clippy::too_many_arguments)]
fn run<R, E>(
    endpoint: Option<String>,
    sample_rate: u32,
    channels: u16,
    mut render: R,
    mut on_error: E,
    commands: mpsc::Receiver<Command>,
    ready: mpsc::SyncSender<Result<(ExclusiveFormat, u32), String>>,
) where
    R: FnMut(ExclusiveBuffer<'_>),
    E: FnMut(String),
{
    if let Err(error) = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.ok() {
        let _ = ready.send(Err(format!("CoInitializeEx failed: {error}")));
        return;
    }
    let _com = ComGuard;
    let opened = match unsafe { open_client(endpoint.as_deref(), sample_rate, channels) } {
        Ok(opened) => opened,
        Err(error) => {
            let _ = ready.send(Err(error));
            return;
        }
    };
    // 启动前先填一整块静音，否则 Start 后第一个周期会播出未初始化的数据
    if let Err(error) = unsafe { prefill_silence(&opened) } {
        let _ = ready.send(Err(error));
        return;
    }
    if ready
        .send(Ok((opened.format, opened.buffer_frames)))
        .is_err()
    {
        return;
    }
    log::info!(
        target: "cpal-output",
        "exclusive output ready: {} Hz, {} ch, {:?} {}/{} bit, buffer={} frames",
        sample_rate,
        channels,
        opened.format.kind,
        opened.format.valid_bits,
        opened.format.container_bits,
        opened.buffer_frames
    );

    let samples = opened.buffer_frames as usize * usize::from(channels);
    let mut running = false;
    loop {
        let command = if running {
            commands.try_recv().ok()
        } else {
            // 暂停时没有事件可等，直接阻塞在命令上
            match commands.recv() {
                Ok(command) => Some(command),
                Err(_) => Some(Command::Stop),
            }
        };
        match command {
            Some(Command::Play) if !running => {
                if let Err(error) = unsafe { opened.client.Start() } {
                    on_error(format!("exclusive Start failed: {error}"));
                    break;
                }
                running = true;
            }
            Some(Command::Pause) if running => {
                let _ = unsafe { opened.client.Stop() };
                running = false;
                continue;
            }
            Some(Command::Stop) => break,
            _ => {}
        }
        if !running {
            continue;
        }
        if unsafe { WaitForSingleObject(opened.event.0, EVENT_WAIT_MS) } != WAIT_OBJECT_0 {
            continue;
        }
        let buffer = match unsafe { opened.render_client.GetBuffer(opened.buffer_frames) } {
            Ok(buffer) => buffer,
            Err(error) => {
                report_stream_error(&mut on_error, "GetBuffer", &error);
                break;
            }
        };
        // SAFETY: GetBuffer 返回的缓冲至少 buffer_frames * block_align 字节，
        // 且按样本大小对齐；在 ReleaseBuffer 之前只由本线程访问
        unsafe {
            match opened.format.kind {
                ExclusiveSampleKind::F32 => render(ExclusiveBuffer::F32(
                    std::slice::from_raw_parts_mut(buffer.cast::<f32>(), samples),
                )),
                ExclusiveSampleKind::I32 => render(ExclusiveBuffer::I32(
                    std::slice::from_raw_parts_mut(buffer.cast::<i32>(), samples),
                )),
                ExclusiveSampleKind::I16 => render(ExclusiveBuffer::I16(
                    std::slice::from_raw_parts_mut(buffer.cast::<i16>(), samples),
                )),
            }
        }
        if let Err(error) = unsafe { opened.render_client.ReleaseBuffer(opened.buffer_frames, 0) } {
            report_stream_error(&mut on_error, "ReleaseBuffer", &error);
            break;
        }
    }
    let _ = unsafe { opened.client.Stop() };
}

fn report_stream_error<E: FnMut(String)>(
    on_error: &mut E,
    step: &str,
    error: &windows::core::Error,
) {
    if error.code() == AUDCLNT_E_DEVICE_INVALIDATED {
        on_error(format!("exclusive output device invalidated during {step}"));
    } else {
        on_error(format!("exclusive {step} failed: {error}"));
    }
}

unsafe fn prefill_silence(opened: &Opened) -> Result<(), String> {
    opened
        .render_client
        .GetBuffer(opened.buffer_frames)
        .map_err(|error| format!("exclusive prefill GetBuffer failed: {error}"))?;
    opened
        .render_client
        .ReleaseBuffer(opened.buffer_frames, AUDCLNT_BUFFERFLAGS_SILENT.0 as u32)
        .map_err(|error| format!("exclusive prefill ReleaseBuffer failed: {error}"))
}

unsafe fn open_client(
    endpoint: Option<&str>,
    sample_rate: u32,
    channels: u16,
) -> Result<Opened, String> {
    let device = find_device(endpoint)?;
    let mut last_error = String::from("No exclusive format accepted by the device");
    for format in EXCLUSIVE_FORMAT_CANDIDATES {
        let wave = wave_format(format, sample_rate, channels);
        let probe = activate(&device)?;
        let supported = probe.IsFormatSupported(
            AUDCLNT_SHAREMODE_EXCLUSIVE,
            &wave.Format as *const WAVEFORMATEX,
            None,
        );
        if supported.is_err() {
            last_error = format!(
                "{:?} {}-bit rejected: {}",
                format.kind,
                format.valid_bits,
                windows::core::Error::from(supported)
            );
            continue;
        }
        match initialize(&device, probe, &wave, format, sample_rate) {
            Ok(opened) => return Ok(opened),
            Err(error) => last_error = error,
        }
    }
    Err(last_error)
}

unsafe fn initialize(
    device: &IMMDevice,
    client: IAudioClient,
    wave: &WAVEFORMATEXTENSIBLE,
    format: ExclusiveFormat,
    sample_rate: u32,
) -> Result<Opened, String> {
    let mut default_period = 0i64;
    client
        .GetDevicePeriod(Some(&mut default_period), None)
        .map_err(|error| format!("GetDevicePeriod failed: {error}"))?;
    let mut client = client;
    let mut period = default_period;
    let mut result = init_exclusive(&client, period, wave);
    if let Err(error) = &result {
        if error.code() == AUDCLNT_E_BUFFER_SIZE_NOT_ALIGNED {
            // 按驱动给出的对齐帧数换算周期，换一个全新的 IAudioClient 重试（官方文档要求）
            let aligned = client
                .GetBufferSize()
                .map_err(|error| format!("GetBufferSize failed: {error}"))?;
            period = frames_to_hns(aligned, sample_rate);
            client = activate(device)?;
            result = init_exclusive(&client, period, wave);
        }
    }
    result.map_err(|error| {
        format!(
            "exclusive Initialize ({:?} {}-bit, period={}hns) failed: {error}",
            format.kind, format.valid_bits, period
        )
    })?;
    let event = CreateEventA(None, false, false, PCSTR::null())
        .map_err(|error| format!("CreateEvent failed: {error}"))?;
    let event = EventHandle(event);
    client
        .SetEventHandle(event.0)
        .map_err(|error| format!("SetEventHandle failed: {error}"))?;
    let buffer_frames = client
        .GetBufferSize()
        .map_err(|error| format!("GetBufferSize failed: {error}"))?;
    let render_client = client
        .GetService::<IAudioRenderClient>()
        .map_err(|error| format!("GetService(IAudioRenderClient) failed: {error}"))?;
    Ok(Opened {
        client,
        render_client,
        event,
        format,
        buffer_frames,
    })
}

unsafe fn init_exclusive(
    client: &IAudioClient,
    period: i64,
    wave: &WAVEFORMATEXTENSIBLE,
) -> windows::core::Result<()> {
    // 事件驱动的独占模式要求缓冲时长与周期相同
    client.Initialize(
        AUDCLNT_SHAREMODE_EXCLUSIVE,
        AUDCLNT_STREAMFLAGS_EVENTCALLBACK,
        period,
        period,
        &wave.Format as *const WAVEFORMATEX,
        None,
    )
}

unsafe fn activate(device: &IMMDevice) -> Result<IAudioClient, String> {
    device
        .Activate::<IAudioClient>(CLSCTX_ALL, None)
        .map_err(|error| format!("Could not activate audio client: {error}"))
}

fn wave_format(format: ExclusiveFormat, sample_rate: u32, channels: u16) -> WAVEFORMATEXTENSIBLE {
    let block_align = format.block_align(channels);
    WAVEFORMATEXTENSIBLE {
        Format: WAVEFORMATEX {
            wFormatTag: WAVE_FORMAT_EXTENSIBLE as u16,
            nChannels: channels,
            nSamplesPerSec: sample_rate,
            nAvgBytesPerSec: sample_rate * u32::from(block_align),
            nBlockAlign: block_align,
            wBitsPerSample: format.container_bits,
            cbSize: (std::mem::size_of::<WAVEFORMATEXTENSIBLE>()
                - std::mem::size_of::<WAVEFORMATEX>()) as u16,
        },
        Samples: WAVEFORMATEXTENSIBLE_0 {
            wValidBitsPerSample: format.valid_bits,
        },
        dwChannelMask: channel_mask(channels),
        SubFormat: match format.kind {
            ExclusiveSampleKind::F32 => KSDATAFORMAT_SUBTYPE_IEEE_FLOAT,
            ExclusiveSampleKind::I32 | ExclusiveSampleKind::I16 => KSDATAFORMAT_SUBTYPE_PCM,
        },
    }
}

unsafe fn find_device(endpoint: Option<&str>) -> Result<IMMDevice, String> {
    let enumerator: IMMDeviceEnumerator =
        CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)
            .map_err(|error| format!("Could not create device enumerator: {error}"))?;
    let Some(name) = endpoint else {
        return enumerator
            .GetDefaultAudioEndpoint(eRender, eConsole)
            .map_err(|error| format!("No default audio output device: {error}"));
    };
    let collection = enumerator
        .EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE)
        .map_err(|error| format!("Could not list audio endpoints: {error}"))?;
    let count = collection
        .GetCount()
        .map_err(|error| format!("Could not count audio endpoints: {error}"))?;
    for index in 0..count {
        let Ok(device) = collection.Item(index) else {
            continue;
        };
        if friendly_name(&device).as_deref() == Some(name) {
            return Ok(device);
        }
    }
    Err("Selected audio output device is unavailable".into())
}

/// 与 cpal 的 Device::name 取同一个属性，保证设置里存的设备名能对上
unsafe fn friendly_name(device: &IMMDevice) -> Option<String> {
    let store = device.OpenPropertyStore(STGM_READ).ok()?;
    let mut value = store
        .GetValue(&DEVPKEY_Device_FriendlyName as *const _ as *const _)
        .ok()?;
    let raw = &value.as_raw().Anonymous.Anonymous;
    let name = if raw.vt == VT_LPWSTR.0 {
        let pointer = raw.Anonymous.pwszVal;
        if pointer.is_null() {
            None
        } else {
            let mut len = 0;
            while *pointer.add(len) != 0 {
                len += 1;
            }
            let wide = std::slice::from_raw_parts(pointer, len);
            Some(OsString::from_wide(wide).to_string_lossy().into_owned())
        }
    } else {
        None
    };
    let _ = PropVariantClear(&mut value);
    name
}
