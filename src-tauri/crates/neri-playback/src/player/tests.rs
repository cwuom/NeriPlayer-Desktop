/// Device names have to split into card and role so the UI can render KDE-style labels
#[cfg(target_os = "linux")]
#[test]
fn linux_output_device_names_split_into_card_and_role() {
    use super::{card_name_from_hint, parse_alsa_cards, pcm_role, split_pcm_name};
    use std::collections::HashMap;

    let cards = parse_alsa_cards(
        " 0 [ONBOARD            ]: HDA-Intel - Example Onboard Audio\n\
         \x20                     Example Onboard Audio at 0x0 irq 0\n\
         \x20 1 [USB0          ]: USB-Audio - Example USB Audio\n\
         \x20 3 [USB1           ]: USB-Audio - Example Surround Audio\n",
    );
    assert_eq!(
        cards.get("USB0").map(String::as_str),
        Some("Example USB Audio")
    );
    assert_eq!(
        cards.get("ONBOARD").map(String::as_str),
        Some("Example Onboard Audio")
    );
    assert_eq!(
        cards.get("USB1").map(String::as_str),
        Some("Example Surround Audio")
    );
    assert_eq!(cards.len(), 3, "缩进的续行不能当成卡片");

    assert_eq!(
        split_pcm_name("front:CARD=USB0,DEV=0"),
        ("front", Some("USB0"), Some("0"))
    );
    assert_eq!(
        split_pcm_name("sysdefault:CARD=ONBOARD"),
        ("sysdefault", Some("ONBOARD"), None)
    );
    assert_eq!(split_pcm_name("default"), ("default", None, None));
    assert_eq!(split_pcm_name("pipewire"), ("pipewire", None, None));

    assert_eq!(pcm_role("front", Some("0")), Some(("analog_stereo", None)));
    assert_eq!(pcm_role("sysdefault", None), Some(("system_default", None)));
    assert_eq!(
        pcm_role("surround51", Some("0")),
        Some(("surround", Some("5.1".into())))
    );
    assert_eq!(
        pcm_role("hdmi", Some("2")),
        Some(("hdmi", Some("2".into())))
    );
    assert_eq!(
        pcm_role("iec958", Some("0")),
        Some(("spdif", Some("0".into())))
    );
    assert_eq!(pcm_role("hw", Some("0")), Some(("hardware", None)));
    assert_eq!(
        pcm_role("pipewire", None),
        None,
        "插件 PCM 没有卡片角色，用描述显示"
    );
    assert_eq!(pcm_role("jack", None), None);

    // The card name prefers /proc/asound/cards, then the description's first line
    let mut missing = HashMap::new();
    let description = "Example USB Audio, USB\nFront output".to_string();
    assert_eq!(
        super::describe_output_device("front:CARD=USB0,DEV=0", Some(&description), &missing).0,
        Some("Example USB Audio".to_string()),
    );
    missing.insert("USB0".to_string(), "来自 /proc 的名字".to_string());
    assert_eq!(
        super::describe_output_device("front:CARD=USB0,DEV=0", Some(&description), &missing).0,
        Some("来自 /proc 的名字".to_string()),
    );
    assert_eq!(
        super::describe_output_device("default", Some(&description), &missing),
        (None, None, None),
    );
    assert_eq!(
        card_name_from_hint("Example Onboard Audio, ALC000 Analog\nDefault Audio Device"),
        Some("Example Onboard Audio".to_string())
    );
}

#[test]
fn android_alignment_stale_device_error_does_not_match_a_rebuilt_session() {
    let previous = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
    let current = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    assert!(!super::matches_output_session(7, &current, 7, &previous));
    assert!(super::matches_output_session(
        7,
        &current,
        7,
        &current.clone()
    ));
    assert!(!super::matches_output_session(8, &current, 7, &current));
}

#[test]
fn android_alignment_abandoned_output_change_is_cancelled() {
    let (_sender, receiver) = std::sync::mpsc::channel();
    let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    drop(super::OutputDeviceChangeRequest {
        receiver,
        cancel: cancel.clone(),
        completed: false,
    });
    assert!(cancel.load(std::sync::atomic::Ordering::Acquire));
}

#[test]
fn android_alignment_output_selection_follows_default_changes() {
    assert_eq!(
        super::effective_output_name(None, &["speaker", "headphones"], Some("headphones")),
        Some("headphones")
    );
    assert_eq!(
        super::effective_output_name(None, &["speaker", "headphones"], Some("speaker")),
        Some("speaker")
    );
}

#[test]
fn android_alignment_output_selection_preserves_an_available_preference() {
    assert_eq!(
        super::effective_output_name(
            Some("speaker"),
            &["speaker", "headphones"],
            Some("headphones")
        ),
        Some("speaker")
    );
}

#[test]
fn output_device_watch_polls_on_playback_start_after_idle_updates() {
    let (sender, receiver) = std::sync::mpsc::channel();
    sender.send(false).unwrap();
    sender.send(false).unwrap();
    sender.send(true).unwrap();
    let mut active = false;
    assert!(super::wait_for_output_device_poll(&receiver, &mut active));
    assert!(active);
}

#[test]
fn output_device_watch_does_not_poll_on_pause_or_duplicate_updates() {
    let (sender, receiver) = std::sync::mpsc::channel();
    sender.send(true).unwrap();
    sender.send(false).unwrap();
    drop(sender);
    let mut active = true;
    assert!(!super::wait_for_output_device_poll(&receiver, &mut active));
    assert!(!active);
}

#[test]
fn output_device_watch_exits_when_the_controller_disconnects() {
    let (sender, receiver) = std::sync::mpsc::channel();
    drop(sender);
    let mut active = false;
    assert!(!super::wait_for_output_device_poll(&receiver, &mut active));
}

#[test]
fn listed_devices_switch_output_only_when_the_target_changes() {
    let available = vec!["speaker".to_string(), "headphones".to_string()];
    assert_eq!(
        super::output_device_to_switch(None, &available, Some("headphones"), Some("headphones")),
        None
    );
    assert_eq!(
        super::output_device_to_switch(None, &available, Some("headphones"), Some("speaker"))
            .as_deref(),
        Some("headphones"),
    );
    assert_eq!(
        super::output_device_to_switch(
            Some("speaker"),
            &available,
            Some("headphones"),
            Some("speaker")
        ),
        None
    );
    assert_eq!(
        super::output_device_to_switch(None, &available, Some("speaker"), None).as_deref(),
        Some("speaker"),
        "还没有设备档案时直接打开目标设备",
    );
    assert_eq!(
        super::output_device_to_switch(None, &[], None, Some("speaker")),
        None
    );
}

#[test]
fn android_alignment_output_selection_falls_back_and_recovers_after_disconnect() {
    assert_eq!(
        super::effective_output_name(Some("headphones"), &["speaker"], Some("speaker")),
        Some("speaker")
    );
    assert_eq!(
        super::effective_output_name(
            Some("headphones"),
            &["speaker", "headphones"],
            Some("speaker")
        ),
        Some("headphones")
    );
    assert_eq!(
        super::effective_output_name(Some("headphones"), &[], None),
        None
    );
}

use super::{
    channel_sample, clamp_position, duration_to_frames, ensure_generation,
    ensure_preparation_current, make_decoder_for_position, take_latest_seek, wait_for_newer_seek,
    AudioCmd, AudioSource, GenerationToken, PlaybackClock, SEEK_SUPERSEDED,
};
use std::collections::VecDeque;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::time::Duration;

#[test]
fn local_file_release_waits_for_decoder_before_file_operations() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("playing.aac");
    std::fs::write(&path, include_bytes!("../fixtures/hls-silence.aac")).unwrap();
    let source = AudioSource::File(path.to_string_lossy().into_owned(), 192);
    let mut decoder = make_decoder_for_position(&source, 0, false).unwrap();
    assert!(decoder.next().is_some());
    let shared = Arc::new(super::PlaybackShared {
        ring: Arc::new(crate::buffered::PcmRing::new(8)),
        channels: 1,
        sample_rate: 48_000,
        paused: std::sync::atomic::AtomicBool::new(true),
        buffering: std::sync::atomic::AtomicBool::new(false),
        finished: std::sync::atomic::AtomicBool::new(false),
        cancelled: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        volume: super::AtomicF32::new(1.0),
        fade_gain: super::AtomicF32::new(1.0),
        speed: super::AtomicF32::new(1.0),
        buffer_target_frames: std::sync::atomic::AtomicUsize::new(1),
        clock: Arc::new(PlaybackClock::new(0)),
        wake_lock: std::sync::Mutex::new(()),
        wake: std::sync::Condvar::new(),
        device_lost: std::sync::atomic::AtomicBool::new(false),
        first_frame_ns: Arc::new(AtomicU64::new(0)),
        stretch_buffered: std::sync::atomic::AtomicUsize::new(0),
        effects: crate::effects::EffectsControl::new_shared(),
        loudness: crate::effects::TrackLoudness::new_shared(),
    });
    let exited = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let worker_shared = Arc::clone(&shared);
    let worker_exited = Arc::clone(&exited);
    let worker = std::thread::spawn(move || {
        while !worker_shared.cancelled.load(Ordering::Acquire) {
            let guard = worker_shared.wake_lock.lock().unwrap();
            let _ = worker_shared
                .wake
                .wait_timeout(guard, Duration::from_millis(20))
                .unwrap();
        }
        std::thread::sleep(Duration::from_millis(80));
        drop(decoder);
        worker_exited.store(true, Ordering::Release);
    });
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        assert!(std::fs::OpenOptions::new()
            .write(true)
            .share_mode(0)
            .open(&path)
            .is_err());
    }
    super::finish_decode_worker(&source, &shared, worker);
    let released = exited.load(Ordering::Acquire);
    // 失败时也让临时文件在解码线程退出后清理
    let deadline = std::time::Instant::now() + Duration::from_secs(1);
    while !exited.load(Ordering::Acquire) && std::time::Instant::now() < deadline {
        std::thread::yield_now();
    }
    assert!(
        released,
        "file release must wait until the decoder has dropped its handle"
    );
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        drop(
            std::fs::OpenOptions::new()
                .write(true)
                .share_mode(0)
                .open(&path)
                .unwrap(),
        );
    }
    let renamed = path.with_file_name("released.aac");
    std::fs::rename(&path, &renamed).unwrap();
    std::fs::remove_file(renamed).unwrap();
}

#[test]
fn local_file_release_matches_only_the_requested_file() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("playing.aac");
    let other = directory.path().join("other.aac");
    std::fs::write(&path, b"audio").unwrap();
    std::fs::write(&other, b"audio").unwrap();
    let source = AudioSource::File(path.to_string_lossy().into_owned(), 0);
    assert!(super::matches_local_file(
        &source,
        &directory.path().join(".").join("playing.aac")
    ));
    assert!(!super::matches_local_file(&source, &other));
    assert!(!super::matches_local_file(
        &AudioSource::Bytes(Arc::from([]), 0),
        &path
    ));
    #[cfg(windows)]
    assert!(super::matches_local_file(
        &source,
        &directory.path().join("PLAYING.AAC")
    ));
}

#[test]
fn local_file_release_does_not_wait_for_a_remote_worker() {
    let buffer = crate::growing::GrowingAudioBuffer::new();
    let source = AudioSource::growing(buffer.reader(), 0);
    let shared = Arc::new(super::PlaybackShared {
        ring: Arc::new(crate::buffered::PcmRing::new(8)),
        channels: 1,
        sample_rate: 48_000,
        paused: std::sync::atomic::AtomicBool::new(false),
        buffering: std::sync::atomic::AtomicBool::new(true),
        finished: std::sync::atomic::AtomicBool::new(false),
        cancelled: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        volume: super::AtomicF32::new(1.0),
        fade_gain: super::AtomicF32::new(1.0),
        speed: super::AtomicF32::new(1.0),
        buffer_target_frames: std::sync::atomic::AtomicUsize::new(1),
        clock: Arc::new(PlaybackClock::new(0)),
        wake_lock: std::sync::Mutex::new(()),
        wake: std::sync::Condvar::new(),
        device_lost: std::sync::atomic::AtomicBool::new(false),
        first_frame_ns: Arc::new(AtomicU64::new(0)),
        stretch_buffered: std::sync::atomic::AtomicUsize::new(0),
        effects: crate::effects::EffectsControl::new_shared(),
        loudness: crate::effects::TrackLoudness::new_shared(),
    });
    let (resume, blocked) = mpsc::channel();
    let (exited, done) = mpsc::channel();
    let worker = std::thread::spawn(move || {
        let _ = blocked.recv_timeout(Duration::from_secs(1));
        let _ = exited.send(());
    });
    super::finish_decode_worker(&source, &shared, worker);
    assert!(matches!(done.try_recv(), Err(mpsc::TryRecvError::Empty)));
    assert!(shared.cancelled.load(Ordering::Acquire));
    let _ = resume.send(());
    done.recv_timeout(Duration::from_secs(1)).unwrap();
}

fn player_without_output(generation: u64) -> (super::PlayerEngine, mpsc::Receiver<AudioCmd>) {
    let (cmd_tx, receiver) = mpsc::channel();
    (
        super::PlayerEngine {
            cmd_tx,
            thread_alive: Arc::new(std::sync::atomic::AtomicBool::new(true)),
            is_playing: true,
            volume: 1.0,
            speed: 1.0,
            current_path: Some("other.aac".into()),
            current_cache_entry: None,
            duration_ms: 192,
            shared_audio_level: crate::analyzer::SharedAudioLevel::new(),
            effects: crate::effects::EffectsControl::new_shared(),
            playback_generation: Arc::new(AtomicU64::new(generation)),
            seek_generation: Arc::new(AtomicU64::new(0)),
            transition_generation: Arc::new(AtomicU64::new(0)),
            loaded_generation: Some(generation),
            ended_transition: None,
            clock: Some(Arc::new(PlaybackClock::new(42))),
            audio_info: None,
        },
        receiver,
    )
}

#[test]
fn natural_end_marker_is_invalidated_by_seek_and_explicit_pause() {
    let (mut player, commands) = player_without_output(8);
    let first_end = player.transition_generation();
    player.mark_ended(first_end);
    assert!(!player.is_playing);
    assert_eq!(player.ended_transition(), Some(first_end));

    let _seek = player.request_seek(60, 8).unwrap();
    assert!(matches!(commands.recv().unwrap(), AudioCmd::Seek { .. }));
    assert!(player.is_playing);
    assert_eq!(player.ended_transition(), None);

    let second_end = player.transition_generation();
    player.mark_ended(second_end);
    assert_eq!(player.ended_transition(), Some(second_end));
    player.pause();
    assert!(matches!(commands.recv().unwrap(), AudioCmd::Pause));
    assert!(!player.is_playing);
    assert_eq!(player.ended_transition(), None);
}

#[test]
fn local_file_release_reply_does_not_clear_a_newer_playback_session() {
    let (mut player, commands) = player_without_output(8);
    let request = player.request_file_release("previous.aac".into()).unwrap();
    let AudioCmd::ReleaseFile { path, reply } = commands.recv().unwrap() else {
        panic!("expected a file release command");
    };
    assert_eq!(path, "previous.aac");
    reply.send(Some(7)).unwrap();
    assert!(player.complete_file_release(request.wait().unwrap()));
    assert!(player.is_playing);
    assert_eq!(player.current_path.as_deref(), Some("other.aac"));
    assert_eq!(player.position_ms(), 42);
    assert!(!player.complete_file_release(None));
    assert!(player.complete_file_release(Some(8)));
    assert!(!player.is_playing);
    assert!(player.current_path.is_none());
    assert_eq!(player.position_ms(), 0);
}

#[test]
fn local_file_release_invalidates_stale_start_without_superseding_a_new_request() {
    let (mut player, _) = player_without_output(7);
    super::invalidate_released_generation(&player.playback_generation, 7);
    assert_eq!(player.playback_generation.load(Ordering::Acquire), 8);
    let started = super::PlaybackStarted {
        duration_ms: 192,
        clock: Arc::new(PlaybackClock::new(0)),
        audio_info: super::SourceAudioInfo::default(),
        cache_entry: None,
    };
    assert!(player
        .complete_start(started, 7, "released.aac".into())
        .is_err());
    assert_eq!(player.current_path.as_deref(), Some("other.aac"));
    super::invalidate_released_generation(&player.playback_generation, 7);
    assert_eq!(player.playback_generation.load(Ordering::Acquire), 8);
}

#[test]
fn stream_audio_info_commits_only_for_the_current_playback_generation() {
    let (mut player, _) = player_without_output(8);
    let decoder = crate::remote::SymphoniaAudioDecoder::new(
        Box::new(std::io::Cursor::new(
            include_bytes!("../fixtures/hls-silence.aac").as_slice(),
        )),
        None,
    )
    .unwrap();
    let source_audio_info = decoder.source_audio_info();
    assert!(player.playback_audio_info(8).is_none());
    let started = super::PlaybackStarted {
        duration_ms: 192,
        clock: Arc::new(PlaybackClock::new(0)),
        audio_info: source_audio_info.clone(),
        cache_entry: None,
    };
    player
        .complete_start(started, 8, "__remote__".into())
        .unwrap();
    let info = player.playback_audio_info(8).unwrap();
    assert_eq!(info, source_audio_info);
    assert!(player.playback_audio_info(7).is_none());
    player.playback_generation.store(9, Ordering::Release);
    assert!(player.playback_audio_info(8).is_none());
    assert!(player.playback_audio_info(9).is_none());
}

#[test]
fn stream_audio_info_stop_and_file_release_remove_loaded_properties() {
    let (mut player, _) = player_without_output(8);
    let decoder = crate::remote::SymphoniaAudioDecoder::new(
        Box::new(std::io::Cursor::new(
            include_bytes!("../fixtures/hls-silence.aac").as_slice(),
        )),
        None,
    )
    .unwrap();
    let started = super::PlaybackStarted {
        duration_ms: 192,
        clock: Arc::new(PlaybackClock::new(0)),
        audio_info: decoder.source_audio_info(),
        cache_entry: None,
    };
    player
        .complete_start(started.clone(), 8, "__remote__".into())
        .unwrap();
    assert!(player.playback_audio_info(8).is_some());
    player.stop();
    assert!(player.playback_audio_info(8).is_none());
    player
        .complete_start(started, 8, "cached.audio".into())
        .unwrap();
    assert!(player.complete_file_release(Some(8)));
    assert!(player.playback_audio_info(8).is_none());
}

#[test]
fn android_alignment_growing_seek_retains_buffer_until_last_source_releases() {
    let buffer = crate::growing::GrowingAudioBuffer::new();
    buffer.append(include_bytes!("../fixtures/hls-silence.aac"));
    buffer.finish();
    let previous = AudioSource::growing(buffer.reader(), 192);
    let seek_source = previous.clone();
    drop(previous);
    assert!(!buffer.is_aborted());
    let cancelled = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let cancellation = crate::remote::RemoteReadCancellation::new(Arc::clone(&cancelled), None);
    let old_decoder_source = seek_source.decoder_source_for_position(0, cancellation);
    let mut old_decoder = make_decoder_for_position(&old_decoder_source, 0, false).unwrap();
    cancelled.store(true, Ordering::Release);
    let _ = old_decoder.next();
    drop(old_decoder);
    drop(old_decoder_source);
    assert!(!buffer.is_aborted());
    let mut next_decoder = make_decoder_for_position(&seek_source, 0, false).unwrap();
    assert!(next_decoder.next().is_some());
    drop(next_decoder);
    drop(seek_source);
    assert!(buffer.is_aborted());
}

#[test]
fn android_alignment_cancelled_growing_prepare_does_not_abort_rollback_source() {
    let buffer = crate::growing::GrowingAudioBuffer::new();
    buffer.append(include_bytes!("../fixtures/hls-silence.aac"));
    buffer.finish();
    let rollback = AudioSource::growing(buffer.reader(), 192);
    let cancelled = Arc::new(std::sync::atomic::AtomicBool::new(true));
    let failed = rollback.decoder_source_for_position(
        0,
        crate::remote::RemoteReadCancellation::new(cancelled, None),
    );
    assert!(make_decoder_for_position(&failed, 0, false).is_err());
    drop(failed);
    assert!(!buffer.is_aborted());
    assert!(make_decoder_for_position(&rollback, 0, false)
        .unwrap()
        .next()
        .is_some());
    drop(rollback);
    assert!(buffer.is_aborted());
}

#[test]
fn android_alignment_growing_reader_uses_session_and_disarmed_prepare_cancellation() {
    use std::io::Read;
    let buffer = crate::growing::GrowingAudioBuffer::new();
    buffer.append(b"abc");
    buffer.finish();
    let source = AudioSource::growing(buffer.reader(), 0);
    let session = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let prepare = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let cancellation = crate::remote::RemoteReadCancellation::new(Arc::clone(&session), None)
        .with_external_cancel(Arc::clone(&prepare));
    let guard = cancellation.clone();
    let AudioSource::Growing(mut reader, _, _) =
        source.decoder_source_for_position(0, cancellation)
    else {
        panic!("expected growing source")
    };
    guard.disarm_operation_guard();
    prepare.store(true, Ordering::Release);
    assert_eq!(reader.read(&mut [0; 1]).unwrap(), 1);
    session.store(true, Ordering::Release);
    assert!(reader.read(&mut [0; 1]).is_err());
    assert!(!buffer.is_aborted());
}

#[test]
fn android_alignment_committed_decode_worker_survives_the_next_seek_generation() {
    let generation = Arc::new(AtomicU64::new(7));
    let seek = Arc::new(AtomicU64::new(1));
    let cancelled = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let cancellation = crate::remote::RemoteReadCancellation::new(
        Arc::clone(&cancelled),
        Some((Arc::clone(&seek), 1)),
    );
    let shared = Arc::new(super::PlaybackShared {
        ring: Arc::new(crate::buffered::PcmRing::new(8)),
        channels: 1,
        sample_rate: 48_000,
        paused: std::sync::atomic::AtomicBool::new(false),
        buffering: std::sync::atomic::AtomicBool::new(true),
        finished: std::sync::atomic::AtomicBool::new(false),
        cancelled,
        volume: super::AtomicF32::new(1.0),
        fade_gain: super::AtomicF32::new(1.0),
        speed: super::AtomicF32::new(1.0),
        buffer_target_frames: std::sync::atomic::AtomicUsize::new(1),
        clock: Arc::new(PlaybackClock::new(0)),
        wake_lock: std::sync::Mutex::new(()),
        wake: std::sync::Condvar::new(),
        device_lost: std::sync::atomic::AtomicBool::new(false),
        first_frame_ns: Arc::new(AtomicU64::new(0)),
        stretch_buffered: std::sync::atomic::AtomicUsize::new(0),
        effects: crate::effects::EffectsControl::new_shared(),
        loudness: crate::effects::TrackLoudness::new_shared(),
    });
    let decoder = crate::remote::SymphoniaAudioDecoder::new(
        Box::new(std::io::Cursor::new(
            include_bytes!("../fixtures/hls-silence.aac").as_slice(),
        )),
        Some("aac"),
    )
    .unwrap();
    let worker = super::spawn_decode_worker(
        Box::new(decoder),
        Arc::clone(&shared),
        crate::analyzer::SharedAudioLevel::new(),
        generation,
        7,
        cancellation.clone(),
    )
    .unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(1);
    while shared.ring.readable_samples() < 8 && std::time::Instant::now() < deadline {
        std::thread::yield_now();
    }
    assert_eq!(shared.ring.readable_samples(), 8);
    cancellation.disarm_operation_guard();
    seek.store(2, Ordering::Release);
    std::thread::sleep(Duration::from_millis(40));
    let kept_running = !shared.finished.load(Ordering::Acquire);
    shared.cancelled.store(true, Ordering::Release);
    shared.wake.notify_all();
    worker.join().unwrap();
    assert!(
        kept_running,
        "a submitted worker must stop through its session token"
    );
}

#[test]
fn duration_to_frames_matches_android_buffer_thresholds() {
    assert_eq!(
        duration_to_frames(Duration::from_millis(800), 48_000),
        38_400
    );
    assert_eq!(
        duration_to_frames(Duration::from_millis(1_500), 48_000),
        72_000
    );
}

#[test]
fn surround_downmix_keeps_center_dialogue_and_never_clips() {
    // 5.1 只有中置有信号：旧做法只取前两个声道，对白会整段消失
    let center_only = [0.0, 0.0, 0.5, 0.0, 0.0, 0.0];
    let stereo = super::Downmix::new(6, 2).expect("5.1 to stereo needs a downmix");
    let (left, right) = (
        stereo.sample(&center_only, 0),
        stereo.sample(&center_only, 1),
    );
    assert!(
        left > 0.1 && (left - right).abs() < 1e-6,
        "L={left} R={right}"
    );

    let lfe_only = [0.0, 0.0, 0.0, 0.9, 0.0, 0.0];
    assert_eq!(
        stereo.sample(&lfe_only, 0),
        0.0,
        "ITU 默认下混不计入低频声道"
    );

    let full_scale = [1.0f32; 6];
    for output in 0..2 {
        assert!(
            stereo.sample(&full_scale, output) <= 1.0 + 1e-6,
            "满幅输入下混后不得削波"
        );
    }
    let mono = super::Downmix::new(6, 1).expect("5.1 to mono needs a downmix");
    assert!(mono.sample(&center_only, 0) > 0.1);
    assert!(mono.sample(&full_scale, 0) <= 1.0 + 1e-6);

    assert!(super::Downmix::new(2, 2).is_none(), "立体声不需要下混");
    assert!(
        super::Downmix::new(2, 1).is_none(),
        "立体声转单声道仍按平均处理"
    );
    assert!(
        super::Downmix::new(6, 6).is_none(),
        "输出声道够多时直接映射"
    );
}

#[test]
fn channel_conversion_handles_mono_and_downmix() {
    assert_eq!(channel_sample(&[0.25], 1, 2), 0.25);
    assert!((channel_sample(&[0.25, 0.75], 0, 1) - 0.5).abs() < f32::EPSILON);
    assert_eq!(channel_sample(&[0.25, 0.75], 1, 2), 0.75);
}

/// 回归（seek 双亡死局）：旧 seek 的 prepare 被更新 seek 的代际递增
/// 打断后，必须能从命令队列接过那条更新的 Seek 完整重跑，
/// 而不是双双失败留下无人重启的空会话
#[test]
fn wait_for_newer_seek_adopts_queued_seek() {
    let (sender, receiver) = mpsc::channel();
    let (reply, _result) = mpsc::channel();
    sender
        .send(AudioCmd::Seek {
            position_ms: 210_641,
            playback_generation: 7,
            seek_generation: 4,
            issued_ns: 0,
            reply,
        })
        .expect("queue newer seek");
    let mut deferred = VecDeque::new();

    let adopted = wait_for_newer_seek(&receiver, &mut deferred, Duration::from_millis(200))
        .expect("newer seek must be adopted");

    assert_eq!(
        (
            adopted.position_ms,
            adopted.playback_generation,
            adopted.seek_generation,
        ),
        (210_641, 7, 4),
    );
    assert!(deferred.is_empty());
}

/// 非 Seek 命令不能打断接管等待：保序暂存后继续等——ticker 的
/// QueryEmpty 会夹在两次拖动之间，第一条非 Seek 就放弃的话
/// 接管几乎必然失败（实机日志：每次接管前多跑一次回滚 prepare）
#[test]
fn wait_for_newer_seek_defers_noise_and_still_adopts() {
    let (sender, receiver) = mpsc::channel();
    let (reply, _result) = mpsc::channel();
    sender
        .send(AudioCmd::SetVolume(0.5))
        .expect("queue non-seek command");
    sender
        .send(AudioCmd::Seek {
            position_ms: 4_606_826,
            playback_generation: 7,
            seek_generation: 9,
            issued_ns: 0,
            reply,
        })
        .expect("queue newer seek behind noise");
    let mut deferred = VecDeque::new();

    let adopted = wait_for_newer_seek(&receiver, &mut deferred, Duration::from_millis(200))
        .expect("seek behind noise must still be adopted");

    assert_eq!(adopted.position_ms, 4_606_826);
    assert_eq!(deferred.len(), 1);
    assert!(matches!(deferred.front(), Some(AudioCmd::SetVolume(_))));
}

/// 宽限超时（极端调度竞态下新 Seek 未入队）：返回 None，
/// 调用方走回滚重建，迟到的 Seek 会在恢复出的会话上正常执行；
/// 期间收到的非 Seek 命令仍保序暂存不丢
#[test]
fn wait_for_newer_seek_times_out_on_empty_queue() {
    let (sender, receiver) = mpsc::channel::<AudioCmd>();
    sender
        .send(AudioCmd::SetVolume(0.5))
        .expect("queue non-seek command");
    let mut deferred = VecDeque::new();

    let started = std::time::Instant::now();
    let adopted = wait_for_newer_seek(&receiver, &mut deferred, Duration::from_millis(30));

    assert!(adopted.is_none());
    assert_eq!(deferred.len(), 1);
    assert!(started.elapsed() < Duration::from_secs(5));
}

/// 已有暂存命令时继续等待新 Seek：暂存命令保序不丢、不被覆盖
#[test]
fn wait_for_newer_seek_preserves_existing_deferred_commands() {
    let (sender, receiver) = mpsc::channel();
    let (reply, _result) = mpsc::channel();
    sender
        .send(AudioCmd::Seek {
            position_ms: 1_000,
            playback_generation: 1,
            seek_generation: 2,
            issued_ns: 0,
            reply,
        })
        .expect("queue seek behind deferred");
    let mut deferred = VecDeque::from([AudioCmd::Pause]);

    let adopted = wait_for_newer_seek(&receiver, &mut deferred, Duration::from_millis(200))
        .expect("existing deferred must not block adoption");

    assert_eq!(adopted.position_ms, 1_000);
    assert_eq!(deferred.len(), 1);
    assert!(matches!(deferred.front(), Some(AudioCmd::Pause)));
}

#[test]
fn seek_queue_keeps_latest_consecutive_position() {
    let (sender, receiver) = mpsc::channel();
    let (first_reply, first_result) = mpsc::channel();
    let (second_reply, second_result) = mpsc::channel();
    let (third_reply, third_result) = mpsc::channel();
    assert!(sender
        .send(AudioCmd::Seek {
            position_ms: 100,
            playback_generation: 7,
            seek_generation: 1,
            issued_ns: 10,
            reply: first_reply,
        })
        .is_ok());
    assert!(sender
        .send(AudioCmd::Seek {
            position_ms: 800,
            playback_generation: 7,
            seek_generation: 2,
            issued_ns: 20,
            reply: second_reply,
        })
        .is_ok());
    assert!(sender
        .send(AudioCmd::Seek {
            position_ms: 1_600,
            playback_generation: 8,
            seek_generation: 3,
            issued_ns: 30,
            reply: third_reply,
        })
        .is_ok());
    let mut latest = 0;
    let mut generation = 7;
    let mut seek_generation = 0;
    let mut issued_ns = 0;
    let (initial_reply, initial_result) = mpsc::channel();
    let mut reply = initial_reply;
    let mut deferred = VecDeque::new();

    take_latest_seek(
        &mut latest,
        &mut generation,
        &mut seek_generation,
        &mut issued_ns,
        &mut reply,
        &receiver,
        &mut deferred,
    );

    assert_eq!(latest, 1_600);
    assert_eq!(generation, 8);
    assert_eq!(seek_generation, 3);
    assert_eq!(issued_ns, 30, "seek 耗时要从最后一次拖动算起");
    assert_eq!(
        initial_result.recv().expect("initial result"),
        Err(SEEK_SUPERSEDED.into())
    );
    assert_eq!(
        first_result.recv().expect("first result"),
        Err(SEEK_SUPERSEDED.into())
    );
    assert_eq!(
        second_result.recv().expect("second result"),
        Err(SEEK_SUPERSEDED.into())
    );
    reply.send(Ok(())).expect("latest reply");
    assert_eq!(third_result.recv().expect("third result"), Ok(()));
    assert!(deferred.is_empty());
}

#[test]
fn playback_generation_rejects_superseded_work() {
    let generation = AtomicU64::new(7);
    assert!(ensure_generation(&generation, 7).is_ok());
    generation.store(8, Ordering::Release);
    assert!(ensure_generation(&generation, 7).is_err());
}

#[test]
fn newer_seek_cancels_only_the_old_seek_preparation() {
    let playback_generation = AtomicU64::new(7);
    let seek_generation = Arc::new(AtomicU64::new(2));
    let old_seek = GenerationToken {
        generation: Arc::clone(&seek_generation),
        expected: 1,
    };
    let current_seek = GenerationToken {
        generation: seek_generation,
        expected: 2,
    };

    assert!(ensure_preparation_current(&playback_generation, 7, Some(&old_seek),).is_err());
    assert!(ensure_preparation_current(&playback_generation, 7, Some(&current_seek),).is_ok());
}

#[test]
fn playback_clock_starts_at_requested_position() {
    let clock = PlaybackClock::new(12_345);
    assert_eq!(clock.position_ms(), 12_345);
}

#[test]
fn known_duration_clamps_seek() {
    assert_eq!(clamp_position(20_000, 10_000), 10_000);
    assert_eq!(clamp_position(20_000, 0), 20_000);
}

fn render_test_shared(channels: usize, capacity_samples: usize) -> Arc<super::PlaybackShared> {
    Arc::new(super::PlaybackShared {
        ring: Arc::new(crate::buffered::PcmRing::new(capacity_samples)),
        channels,
        sample_rate: 48_000,
        paused: std::sync::atomic::AtomicBool::new(false),
        buffering: std::sync::atomic::AtomicBool::new(false),
        finished: std::sync::atomic::AtomicBool::new(false),
        cancelled: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        volume: super::AtomicF32::new(1.0),
        fade_gain: super::AtomicF32::new(1.0),
        speed: super::AtomicF32::new(1.0),
        buffer_target_frames: std::sync::atomic::AtomicUsize::new(1),
        clock: Arc::new(PlaybackClock::new(0)),
        wake_lock: std::sync::Mutex::new(()),
        wake: std::sync::Condvar::new(),
        device_lost: std::sync::atomic::AtomicBool::new(false),
        first_frame_ns: Arc::new(AtomicU64::new(0)),
        stretch_buffered: std::sync::atomic::AtomicUsize::new(0),
        effects: crate::effects::EffectsControl::new_shared(),
        loudness: crate::effects::TrackLoudness::new_shared(),
    })
}

#[test]
fn full_decode_buffer_refills_after_output_consumes_frames() {
    let shared = render_test_shared(1, 8);
    let decoder = crate::remote::SymphoniaAudioDecoder::new(
        Box::new(std::io::Cursor::new(
            include_bytes!("../fixtures/hls-silence.aac").as_slice(),
        )),
        Some("aac"),
    )
    .unwrap();
    let cancellation =
        crate::remote::RemoteReadCancellation::new(Arc::clone(&shared.cancelled), None);
    let worker = super::spawn_decode_worker(
        Box::new(decoder),
        Arc::clone(&shared),
        crate::analyzer::SharedAudioLevel::new(),
        Arc::new(AtomicU64::new(7)),
        7,
        cancellation,
    )
    .unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(1);
    while shared.ring.readable_samples() < 8 && std::time::Instant::now() < deadline {
        std::thread::yield_now();
    }
    let filled = shared.ring.readable_samples() == 8;
    let mut consumed = [0.0; 8];
    let taken = shared.ring.pop_frames(&mut consumed, 1);
    let deadline = std::time::Instant::now() + Duration::from_secs(1);
    while shared.ring.readable_samples() < 8 && std::time::Instant::now() < deadline {
        std::thread::yield_now();
    }
    let refilled = shared.ring.readable_samples() == 8;
    shared.cancelled.store(true, Ordering::Release);
    shared.wake.notify_all();
    worker.join().unwrap();
    assert!(filled);
    assert_eq!(taken, 8);
    assert!(
        refilled,
        "output consumption must resume decoding without a notification"
    );
}

#[test]
fn full_paused_decode_buffer_observes_a_new_generation() {
    let shared = render_test_shared(1, 8);
    shared.paused.store(true, Ordering::Release);
    let generation = Arc::new(AtomicU64::new(7));
    let decoder = crate::remote::SymphoniaAudioDecoder::new(
        Box::new(std::io::Cursor::new(
            include_bytes!("../fixtures/hls-silence.aac").as_slice(),
        )),
        Some("aac"),
    )
    .unwrap();
    let cancellation =
        crate::remote::RemoteReadCancellation::new(Arc::clone(&shared.cancelled), None);
    let worker = super::spawn_decode_worker(
        Box::new(decoder),
        Arc::clone(&shared),
        crate::analyzer::SharedAudioLevel::new(),
        Arc::clone(&generation),
        7,
        cancellation,
    )
    .unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(1);
    while shared.ring.readable_samples() < 8 && std::time::Instant::now() < deadline {
        std::thread::yield_now();
    }
    let filled = shared.ring.readable_samples() == 8;
    generation.store(8, Ordering::Release);
    let deadline = std::time::Instant::now() + Duration::from_secs(1);
    while !worker.is_finished() && std::time::Instant::now() < deadline {
        std::thread::yield_now();
    }
    let superseded = worker.is_finished();
    shared.cancelled.store(true, Ordering::Release);
    shared.wake.notify_all();
    worker.join().unwrap();
    assert!(filled);
    assert!(
        superseded,
        "generation changes must cancel a full paused buffer without a notification"
    );
}

/// 与 build_typed_stream 里一样的回调状态
struct CallbackState {
    chain: super::OutputChain,
}

impl CallbackState {
    fn new(shared: &super::PlaybackShared) -> Self {
        Self {
            chain: super::OutputChain::new(shared),
        }
    }

    fn render(
        &mut self,
        shared: &super::PlaybackShared,
        output: &mut [f32],
        counters: &crate::metrics::OutputMetrics,
    ) {
        super::render_output(shared, output, &mut self.chain, counters);
    }
}

#[test]
fn render_output_counts_a_mid_buffer_underrun_once() {
    let shared = render_test_shared(2, 64);
    for index in 0..3 {
        let value = 0.1 * (index + 1) as f32;
        assert!(shared.ring.try_push_frame(&[value, -value]));
    }
    let counters = crate::metrics::OutputMetrics::new();
    let mut callback = CallbackState::new(&shared);
    let mut output = [1.0f32; 16];

    callback.render(&shared, &mut output, &counters);

    assert!((output[4] - 0.3).abs() < 1e-6 && (output[5] + 0.3).abs() < 1e-6);
    assert!(
        output[6..].iter().all(|sample| *sample == 0.0),
        "取空后整块补静音"
    );
    let snapshot = counters.snapshot();
    assert_eq!(
        (
            snapshot.underruns,
            snapshot.underrun_frames,
            snapshot.rendered_frames
        ),
        (1, 5, 3)
    );
    assert!(shared.buffering.load(Ordering::Acquire), "欠载后进入重缓冲");
    assert_eq!(shared.clock.position_us.load(Ordering::Acquire), 62);

    callback.render(&shared, &mut output, &counters);
    assert_eq!(
        counters.snapshot().underruns,
        1,
        "重缓冲期间的静音不重复计数"
    );
}

#[test]
fn render_output_does_not_count_the_drain_after_the_decoder_finished() {
    let shared = render_test_shared(2, 64);
    assert!(shared.ring.try_push_frame(&[0.5, 0.5]));
    shared.finished.store(true, Ordering::Release);
    let counters = crate::metrics::OutputMetrics::new();
    let mut callback = CallbackState::new(&shared);
    let mut output = [0.0f32; 8];

    callback.render(&shared, &mut output, &counters);

    let snapshot = counters.snapshot();
    assert_eq!(
        (
            snapshot.underruns,
            snapshot.underrun_frames,
            snapshot.rendered_frames
        ),
        (0, 0, 1)
    );
    assert_eq!(
        shared.stretch_buffered.load(Ordering::Acquire),
        0,
        "播完时变速器里不能有残留"
    );
}

#[test]
fn render_output_stays_silent_and_uncounted_while_paused() {
    let shared = render_test_shared(2, 64);
    assert!(shared.ring.try_push_frame(&[0.5, 0.5]));
    shared.paused.store(true, Ordering::Release);
    let counters = crate::metrics::OutputMetrics::new();
    let mut callback = CallbackState::new(&shared);
    let mut output = [1.0f32; 8];

    callback.render(&shared, &mut output, &counters);

    assert!(output.iter().all(|sample| *sample == 0.0));
    assert_eq!(shared.ring.readable_samples(), 2, "暂停时不消费 ring");
    assert_eq!(counters.snapshot().underruns, 0);
    assert_eq!(shared.first_frame_ns.load(Ordering::Acquire), 0);
}

#[test]
fn render_output_records_only_the_first_frame_time() {
    let shared = render_test_shared(1, 64);
    for _ in 0..8 {
        assert!(shared.ring.try_push_frame(&[0.25]));
    }
    let counters = crate::metrics::OutputMetrics::new();
    let mut callback = CallbackState::new(&shared);
    let mut output = [0.0f32; 4];

    callback.render(&shared, &mut output, &counters);
    let first = shared.first_frame_ns.load(Ordering::Acquire);
    assert_ne!(first, 0);
    std::thread::sleep(Duration::from_millis(2));
    callback.render(&shared, &mut output, &counters);

    assert_eq!(shared.first_frame_ns.load(Ordering::Acquire), first);
    assert_eq!(counters.snapshot().rendered_frames, 8);
}

/// 一起听的软同步把倍速调到 1.05：回调里立即按新倍速消费 ring、推进时钟，不停下会话
#[test]
fn speed_changes_take_effect_inside_the_callback_without_a_gap() {
    let shared = render_test_shared(1, 96_000);
    for index in 0..96_000 {
        let sample = (0.5 * (std::f64::consts::TAU * 440.0 * index as f64 / 48_000.0).sin()) as f32;
        assert!(shared.ring.try_push_frame(&[sample]));
    }
    let counters = crate::metrics::OutputMetrics::new();
    let mut callback = CallbackState::new(&shared);
    let mut output = [0.0f32; 480];
    for _ in 0..20 {
        callback.render(&shared, &mut output, &counters);
    }
    let clock_before = shared.clock.position_us.load(Ordering::Acquire);
    let ring_before = shared.ring.readable_samples();
    assert_eq!(clock_before, 200_000, "1 倍速时 9 600 帧对应 200 ms");

    shared.speed.store(1.05);
    for _ in 0..100 {
        callback.render(&shared, &mut output, &counters);
    }
    let clock_advance = shared.clock.position_us.load(Ordering::Acquire) - clock_before;
    let consumed = ring_before - shared.ring.readable_samples();
    assert!(
        (clock_advance as f64 - 1_050_000.0).abs() < 25_000.0,
        "1 秒输出应推进约 1.05 秒媒体时间: {clock_advance}"
    );
    assert!(
        consumed > 48_000,
        "倍速后每秒应消费多于 48 000 帧: {consumed}"
    );
    assert_eq!(counters.snapshot().underruns, 0, "变速不能造成断音");
    assert!(
        output.iter().any(|sample| sample.abs() > 0.1),
        "变速后仍在出声"
    );
}

#[test]
fn vinyl_output_chain_uses_speed_for_pitch_and_finishes_at_exact_media_time() {
    for speed in [0.1f32, 0.95, 1.05, 4.0] {
        let shared = render_test_shared(1, 48_000);
        for index in 0..48_000 {
            let sample =
                (0.5 * (std::f64::consts::TAU * 440.0 * index as f64 / 48_000.0).sin()) as f32;
            assert!(shared.ring.try_push_frame(&[sample]));
        }
        shared.speed.store(speed);
        shared.effects.update(|settings| {
            settings.pitch_centi = 85;
            settings.vinyl_effect_enabled = true;
        });
        shared.finished.store(true, Ordering::Release);
        let counters = crate::metrics::OutputMetrics::new();
        let mut callback = CallbackState::new(&shared);
        let mut block = [0.0; 480];
        let mut output = Vec::new();
        for _ in 0..1500 {
            let before = counters.snapshot().rendered_frames;
            callback.render(&shared, &mut block, &counters);
            let frames = (counters.snapshot().rendered_frames - before) as usize;
            output.extend_from_slice(&block[..frames]);
            if shared.ring.readable_samples() == 0
                && shared.stretch_buffered.load(Ordering::Acquire) == 0
            {
                break;
            }
        }
        let middle = &output[output.len() / 4..output.len() * 3 / 4];
        let crossings = middle
            .windows(2)
            .filter(|pair| pair[0] < 0.0 && pair[1] >= 0.0)
            .count();
        let frequency = crossings as f64 / (middle.len() as f64 / 48_000.0);
        assert!(
            (frequency - 440.0 * f64::from(speed)).abs() < 12.0,
            "speed {speed}: {frequency} Hz"
        );
        assert!(
            output
                .len()
                .abs_diff((48_000.0 / f64::from(speed)).round() as usize)
                <= 3
        );
        assert!(
            shared
                .clock
                .position_us
                .load(Ordering::Acquire)
                .abs_diff(1_000_000)
                <= 1
        );
        assert_eq!(shared.effects.snapshot().pitch_centi, 85);
        assert_eq!(shared.stretch_buffered.load(Ordering::Acquire), 0);
        assert_eq!(counters.snapshot().underruns, 0);
    }
}

#[test]
fn vinyl_switches_restore_independent_pitch_and_remain_continuous() {
    let shared = render_test_shared(1, 480_000);
    for index in 0..480_000 {
        let sample = (0.5 * (std::f64::consts::TAU * 440.0 * index as f64 / 48_000.0).sin()) as f32;
        assert!(shared.ring.try_push_frame(&[sample]));
    }
    shared.speed.store(1.05);
    shared.effects.update(|settings| settings.pitch_centi = 85);
    let counters = crate::metrics::OutputMetrics::new();
    let mut callback = CallbackState::new(&shared);
    let mut output = [0.0; 480];
    for enabled in [false, true, false] {
        shared
            .effects
            .update(|settings| settings.vinyl_effect_enabled = enabled);
        let mut steady = Vec::new();
        let before_clock = shared.clock.position_us.load(Ordering::Acquire);
        let mut previous = output[479];
        let mut max_step = 0.0f32;
        for index in 0..100 {
            callback.render(&shared, &mut output, &counters);
            for sample in output {
                assert!(sample.is_finite());
                max_step = max_step.max((sample - previous).abs());
                previous = sample;
            }
            if index >= 50 {
                steady.extend_from_slice(&output);
            }
        }
        let crossings = steady
            .windows(2)
            .filter(|pair| pair[0] < 0.0 && pair[1] >= 0.0)
            .count();
        let expected = 440.0 * if enabled { 1.05 } else { 0.85 };
        assert!((crossings as f64 * 2.0 - expected).abs() < 12.0);
        assert!(
            max_step < 0.08,
            "vinyl={enabled}: max sample step {max_step}"
        );
        let advanced = shared.clock.position_us.load(Ordering::Acquire) - before_clock;
        assert!((advanced as f64 - 1_050_000.0).abs() < 40_000.0);
        assert_eq!(shared.effects.snapshot().pitch_centi, 85);
    }
    assert_eq!(counters.snapshot().underruns, 0);
}

#[test]
fn pitch_changes_inside_the_callback_keep_the_media_clock_and_audio_continuous() {
    let shared = render_test_shared(1, 144_000);
    for index in 0..144_000 {
        let sample = (0.5 * (std::f64::consts::TAU * 440.0 * index as f64 / 48_000.0).sin()) as f32;
        assert!(shared.ring.try_push_frame(&[sample]));
    }
    let counters = crate::metrics::OutputMetrics::new();
    let mut callback = CallbackState::new(&shared);
    let mut output = [0.0f32; 480];
    for _ in 0..20 {
        callback.render(&shared, &mut output, &counters);
    }
    let clock_before = shared.clock.position_us.load(Ordering::Acquire);
    shared.effects.update(|settings| settings.pitch_centi = 200);
    let mut steady = Vec::new();
    for index in 0..100 {
        callback.render(&shared, &mut output, &counters);
        if index >= 50 {
            steady.extend_from_slice(&output);
        }
    }
    let clock_advance = shared.clock.position_us.load(Ordering::Acquire) - clock_before;
    assert!(
        (clock_advance as f64 - 1_000_000.0).abs() < 40_000.0,
        "pitch must not double media time: {clock_advance}"
    );
    let crossings = steady
        .windows(2)
        .filter(|pair| pair[0] < 0.0 && pair[1] >= 0.0)
        .count();
    assert!((crossings as f64 * 2.0 - 880.0).abs() < 8.0);
    assert_eq!(counters.snapshot().underruns, 0);
}

#[test]
fn output_chain_drains_pitch_buffers_before_finishing_at_the_exact_media_time() {
    for (speed, pitch_centi) in [(0.1f32, 200), (4.0, 25), (1.0, 50), (1.0, 200)] {
        let shared = render_test_shared(1, 48_000);
        for index in 0..48_000 {
            let sample =
                (0.5 * (std::f64::consts::TAU * 440.0 * index as f64 / 48_000.0).sin()) as f32;
            assert!(shared.ring.try_push_frame(&[sample]));
        }
        shared.speed.store(speed);
        shared
            .effects
            .update(|settings| settings.pitch_centi = pitch_centi);
        shared.finished.store(true, Ordering::Release);
        let counters = crate::metrics::OutputMetrics::new();
        let mut callback = CallbackState::new(&shared);
        let mut output = [0.0f32; 480];
        let mut rendered_callbacks = 0;
        for _ in 0..1_500 {
            callback.render(&shared, &mut output, &counters);
            rendered_callbacks += 1;
            if shared.ring.readable_samples() == 0
                && shared.stretch_buffered.load(Ordering::Acquire) == 0
            {
                break;
            }
        }
        assert!(rendered_callbacks > 0);
        assert_eq!(shared.ring.readable_samples(), 0);
        assert_eq!(shared.stretch_buffered.load(Ordering::Acquire), 0);
        let clock = shared.clock.position_us.load(Ordering::Acquire);
        assert!(
            clock.abs_diff(1_000_000) <= 1,
            "speed {speed}, pitch {pitch_centi}: clock {clock}"
        );
        assert_eq!(counters.snapshot().underruns, 0);
    }
}

#[test]
fn rebuilding_an_output_chain_reuses_pitch_without_carrying_the_old_audio() {
    let first = render_test_shared(1, 48_000);
    first.effects.update(|settings| settings.pitch_centi = 150);
    assert!(first.ring.try_push_frame(&[0.75]));
    let mut callback = CallbackState::new(&first);
    let counters = crate::metrics::OutputMetrics::new();
    let mut output = [0.0f32; 480];
    first.finished.store(true, Ordering::Release);
    callback.render(&first, &mut output, &counters);

    let mut rebuilt = render_test_shared(1, 48_000);
    Arc::get_mut(&mut rebuilt).unwrap().effects = Arc::clone(&first.effects);
    rebuilt.clock.store_ms(12_345);
    for index in 0..48_000 {
        let sample =
            (0.25 * (std::f64::consts::TAU * 440.0 * index as f64 / 48_000.0).sin()) as f32;
        assert!(rebuilt.ring.try_push_frame(&[sample]));
    }
    let mut callback = CallbackState::new(&rebuilt);
    let mut steady = Vec::new();
    for index in 0..60 {
        callback.render(&rebuilt, &mut output, &counters);
        if index >= 30 {
            steady.extend_from_slice(&output);
        }
    }
    let crossings = steady
        .windows(2)
        .filter(|pair| pair[0] < 0.0 && pair[1] >= 0.0)
        .count();
    assert!((crossings as f64 / 0.3 - 660.0).abs() < 8.0);
    assert!(steady.iter().all(|sample| sample.abs() < 0.3));
    let position = rebuilt.clock.position_us.load(Ordering::Acquire);
    assert!((position as f64 - 12_945_000.0).abs() < 40_000.0);
}

/// 变速中欠载：变速器里已取出的前瞻要保留，恢复后接着放；播完时时钟与输入时长一致
#[test]
fn an_underrun_while_stretching_keeps_the_buffered_audio() {
    let total = 48_000usize;
    let shared = render_test_shared(1, total);
    let sample = |index: usize| {
        (0.5 * (std::f64::consts::TAU * 440.0 * index as f64 / 48_000.0).sin()) as f32
    };
    for index in 0..9_600 {
        assert!(shared.ring.try_push_frame(&[sample(index)]));
    }
    shared.speed.store(1.25);
    let counters = crate::metrics::OutputMetrics::new();
    let mut callback = CallbackState::new(&shared);
    let mut output = [0.0f32; 480];
    for _ in 0..100 {
        if shared.buffering.load(Ordering::Acquire) {
            break;
        }
        callback.render(&shared, &mut output, &counters);
    }
    assert!(
        shared.buffering.load(Ordering::Acquire),
        "ring 取空后应进入重缓冲"
    );
    assert!(
        shared.stretch_buffered.load(Ordering::Acquire) > 0,
        "欠载时变速器里还留着前瞻"
    );

    for index in 9_600..total {
        assert!(shared.ring.try_push_frame(&[sample(index)]));
    }
    shared.buffering.store(false, Ordering::Release);
    shared.finished.store(true, Ordering::Release);
    for _ in 0..200 {
        callback.render(&shared, &mut output, &counters);
        if shared.ring.readable_samples() == 0
            && shared.stretch_buffered.load(Ordering::Acquire) == 0
        {
            break;
        }
    }
    assert_eq!(
        shared.stretch_buffered.load(Ordering::Acquire),
        0,
        "播完时变速器里不能有残留"
    );
    let position_us = shared.clock.position_us.load(Ordering::Acquire) as f64;
    assert!(
        (position_us - 1_000_000.0).abs() < 1_000.0,
        "1 秒输入播完，时钟应走到 1 秒: {position_us}"
    );
}

/// 每次回调的媒体时长不是整微秒（441 帧 = 9 187.5 µs）：零头要留到下次，不能逐次截掉
#[test]
fn the_clock_keeps_sub_microsecond_remainders_across_callbacks() {
    let shared = render_test_shared(1, 441 * 100);
    for _ in 0..441 * 100 {
        assert!(shared.ring.try_push_frame(&[0.25]));
    }
    let counters = crate::metrics::OutputMetrics::new();
    let mut callback = CallbackState::new(&shared);
    let mut output = [0.0f32; 441];
    for _ in 0..100 {
        callback.render(&shared, &mut output, &counters);
    }
    assert_eq!(shared.clock.position_us.load(Ordering::Acquire), 918_750);
}

/// 播放中打开音量均衡：回调下一块就开始过渡，会话不重建、时钟连续、不断音
#[test]
fn normalization_toggles_inside_the_callback_without_a_rebuild() {
    let shared = render_test_shared(2, 2 * 48_000);
    let mut meter = crate::effects::LoudnessMeter::new(Arc::clone(&shared.loudness));
    for _ in 0..48_000 {
        assert!(shared.ring.try_push_frame(&[0.01, 0.01]));
        meter.observe(&[0.01, 0.01]);
    }
    meter.flush();
    let counters = crate::metrics::OutputMetrics::new();
    let mut callback = CallbackState::new(&shared);
    let mut output = [0.0f32; 960];
    callback.render(&shared, &mut output, &counters);
    assert!(
        output.iter().all(|sample| *sample == 0.01),
        "关着时逐位直通"
    );

    shared
        .effects
        .update(|settings| settings.normalize_volume = true);
    for _ in 0..10 {
        callback.render(&shared, &mut output, &counters);
    }

    let boosted = output[output.len() - 1] / 0.01;
    assert!(
        (boosted - 1.995).abs() < 0.01,
        "打开后应抬到 +6 dB: {boosted}"
    );
    assert_eq!(
        shared.clock.position_us.load(Ordering::Acquire),
        110_000,
        "11 块各 10 ms，时钟连续"
    );
    let snapshot = counters.snapshot();
    assert_eq!(snapshot.underruns, 0);
    assert_eq!(snapshot.normalization_gain_mb, 600, "指标里能看到当前增益");
}

#[test]
fn loudness_is_measured_only_on_channels_that_carry_the_source() {
    assert_eq!(
        super::loudness_channels(2, 8),
        2,
        "立体声接 7.1：多出来的零声道不算"
    );
    assert_eq!(
        super::loudness_channels(1, 2),
        1,
        "单声道复制到两边，统计一份"
    );
    assert_eq!(
        super::loudness_channels(6, 2),
        2,
        "5.1 缩混成立体声后两个声道都有内容"
    );
    assert_eq!(super::loudness_channels(8, 8), 8);
    assert_eq!(super::loudness_channels(0, 2), 1);
}

/// 16 bit 立体声 440 Hz WAV，由若干段（幅度, 秒数）拼成；同时返回解码后应得到的样本
fn sine_sections_wav(sample_rate: u32, sections: &[(f64, u32)]) -> (Vec<u8>, Vec<f32>) {
    let mut samples = Vec::new();
    let mut index = 0usize;
    for &(amplitude, seconds) in sections {
        for _ in 0..(sample_rate * seconds) {
            let phase = std::f64::consts::TAU * 440.0 * index as f64 / f64::from(sample_rate);
            let value = (amplitude * phase.sin() * 32_767.0).round() as i16;
            samples.extend([value, value]);
            index += 1;
        }
    }
    let data_len = (samples.len() * 2) as u32;
    let mut bytes = Vec::with_capacity(44 + data_len as usize);
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(36 + data_len).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16u32.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&2u16.to_le_bytes());
    bytes.extend_from_slice(&sample_rate.to_le_bytes());
    bytes.extend_from_slice(&(sample_rate * 4).to_le_bytes());
    bytes.extend_from_slice(&4u16.to_le_bytes());
    bytes.extend_from_slice(&16u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&data_len.to_le_bytes());
    for sample in &samples {
        bytes.extend_from_slice(&sample.to_le_bytes());
    }
    let decoded = samples
        .iter()
        .map(|sample| f32::from(*sample) / 32_768.0)
        .collect();
    (bytes, decoded)
}

/// 前一半安静、后一半响
fn quiet_then_loud_wav(sample_rate: u32, seconds_each: u32) -> (Vec<u8>, Vec<f32>) {
    sine_sections_wav(sample_rate, &[(0.05, seconds_each), (0.5, seconds_each)])
}

fn wav_file(directory: &tempfile::TempDir, name: &str, bytes: &[u8]) -> AudioSource {
    let path = directory.path().join(name);
    std::fs::write(&path, bytes).unwrap();
    AudioSource::File(path.to_string_lossy().into_owned(), 0)
}

fn target_db(stats: &crate::effects::LoudnessStats) -> f64 {
    20.0 * stats.target_gain().unwrap().log10()
}

/// 安静-响-安静的 40 秒曲子：抽样估计与完整扫描相差不到 0.5 dB，
/// 而只看开头 10 秒的估计差了 6 dB 以上
#[test]
fn quick_estimate_lands_close_to_the_full_scan() {
    let (wav, decoded) = sine_sections_wav(24_000, &[(0.05, 10), (0.5, 20), (0.05, 10)]);
    let directory = tempfile::tempdir().unwrap();
    let source = wav_file(&directory, "sections.wav", &wav);
    let cancel = AtomicBool::new(false);

    let estimate = super::estimate_track_loudness(&source, 24_000, 2, &cancel)
        .unwrap()
        .expect("40 秒的本地文件应给出估计");
    let (full, _) = super::scan_track_loudness(&source, 24_000, 2, &cancel).unwrap();

    assert!(
        (target_db(&estimate) - target_db(&full)).abs() < 0.5,
        "估计 {:.2} dB，整首 {:.2} dB",
        target_db(&estimate),
        target_db(&full),
    );
    assert!(
        estimate.samples() < full.samples() / 3,
        "估计只解码一小部分"
    );
    let mut opening = crate::effects::LoudnessScanner::default();
    opening.observe(&decoded[..decoded.len() / 4]);
    assert!(target_db(&opening.finish()) - target_db(&full) > 6.0);
}

/// 在真实曲库上量快速估计与完整扫描的耗时和差距（发布构建才有参考价值）：
/// `NERI_LOUDNESS_BENCH_DIR=<目录> cargo test --release --lib -- --ignored loudness_scan_timing --nocapture`
#[test]
#[ignore = "needs NERI_LOUDNESS_BENCH_DIR pointing at a music folder"]
fn loudness_scan_timing_on_a_local_library() {
    let directory = std::env::var("NERI_LOUDNESS_BENCH_DIR")
        .expect("set NERI_LOUDNESS_BENCH_DIR to a folder with audio files");
    const AUDIO: [&str; 9] = [
        "mp3", "flac", "m4a", "aac", "ogg", "opus", "wav", "ape", "wv",
    ];
    let mut files: Vec<_> = std::fs::read_dir(directory)
        .unwrap()
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| {
            path.extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| AUDIO.contains(&extension.to_ascii_lowercase().as_str()))
        })
        .collect();
    files.sort();
    let cancel = AtomicBool::new(false);
    for path in files.iter().take(8) {
        let source = AudioSource::File(path.to_string_lossy().into_owned(), 0);
        let started = std::time::Instant::now();
        let estimate = super::estimate_track_loudness(&source, 48_000, 2, &cancel);
        let estimate_ms = started.elapsed().as_millis();
        let started = std::time::Instant::now();
        let full = super::scan_track_loudness(&source, 48_000, 2, &cancel);
        let full_ms = started.elapsed().as_millis();
        let estimate_db = estimate.ok().flatten().map(|stats| target_db(&stats));
        let (full_db, seconds) = match &full {
            Ok((stats, frames)) => (Some(target_db(stats)), *frames as f64 / 48_000.0),
            Err(_) => (None, 0.0),
        };
        println!(
            "{:>6.1}s estimate={estimate_db:>8.2?} dB in {estimate_ms:>5} ms, full={full_db:>8.2?} dB in {full_ms:>6} ms  {}",
            seconds,
            path.file_name().unwrap_or_default().to_string_lossy(),
        );
    }
}

#[test]
fn short_tracks_skip_the_quick_estimate() {
    let (wav, _) = quiet_then_loud_wav(24_000, 2);
    let directory = tempfile::tempdir().unwrap();
    let source = wav_file(&directory, "short.wav", &wav);

    let estimate =
        super::estimate_track_loudness(&source, 24_000, 2, &AtomicBool::new(false)).unwrap();

    assert!(estimate.is_none());
}

/// 起播等估计：估计一到就返回，等不到就在截止时刻放弃，播放请求作废时立刻退出
#[test]
fn waiting_for_the_estimate_is_bounded() {
    let generation = AtomicU64::new(3);
    let not_cancelled = AtomicBool::new(false);
    let track = crate::effects::TrackLoudness::new_shared();
    track.set_estimate_pending(true);
    let landing = Arc::clone(&track);
    let started = std::time::Instant::now();
    let lander = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(30));
        landing.set_estimate_pending(false);
    });
    super::wait_for_loudness_estimate(
        &track,
        started + Duration::from_secs(5),
        &generation,
        3,
        None,
        &not_cancelled,
    )
    .unwrap();
    lander.join().unwrap();
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "估计到了就该返回"
    );

    track.set_estimate_pending(true);
    let started = std::time::Instant::now();
    super::wait_for_loudness_estimate(
        &track,
        started + Duration::from_millis(40),
        &generation,
        3,
        None,
        &not_cancelled,
    )
    .unwrap();
    assert!(started.elapsed() >= Duration::from_millis(40));

    generation.store(4, Ordering::Release);
    assert!(super::wait_for_loudness_estimate(
        &track,
        std::time::Instant::now() + Duration::from_secs(5),
        &generation,
        3,
        None,
        &not_cancelled,
    )
    .is_err());
}

/// 整首扫描读完整个文件，结果与按播放口径统计同样的样本一致，并把目标固定下来；
/// 只看开头安静段的估计比整首高得多——这就是播放时统计在开头的漂移
#[test]
fn loudness_scan_measures_the_whole_file_like_playback() {
    let (wav, decoded) = quiet_then_loud_wav(48_000, 2);
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("scan.wav");
    std::fs::write(&path, &wav).unwrap();
    let source = AudioSource::File(path.to_string_lossy().into_owned(), 0);

    let (stats, frames) =
        super::scan_track_loudness(&source, 48_000, 2, &AtomicBool::new(false)).unwrap();

    assert_eq!(frames, 192_000);
    let mut expected = crate::effects::LoudnessScanner::default();
    expected.observe(&decoded);
    let target = stats.target_gain().unwrap();
    assert!((target - expected.finish().target_gain().unwrap()).abs() < 1e-4);
    let mut opening = crate::effects::LoudnessScanner::default();
    opening.observe(&decoded[..decoded.len() / 2]);
    let early = opening.finish().target_gain().unwrap();
    assert!(
        early / target > 2.0,
        "开头的估计 {early:.3} 应远高于整首 {target:.3}"
    );

    let track = crate::effects::TrackLoudness::new_shared();
    track.set_estimate_pending(true);
    super::run_loudness_scan(
        &source,
        48_000,
        2,
        &AtomicBool::new(false),
        &Arc::downgrade(&track),
        1,
        true,
    );
    assert!(track.is_complete());
    assert!(!track.estimate_pending(), "跳过估计时也要放行等待中的起播");
    assert_eq!(track.analyzed_samples(), stats.samples());
}

/// 字节跳转从分片开头解码：丢掉分片起点到目标之间的音频后，下一个样本正好是目标位置
#[test]
fn virtual_body_start_discards_audio_before_the_target() {
    let (wav, expected) = sine_sections_wav(8_000, &[(0.5, 3)]);
    let source = AudioSource::Bytes(Arc::from(wav), 0);
    let mut decoder = super::make_decoder_for_position(&source, 0, false).unwrap();

    // 分片从 1s 开始，目标 2.25s：丢 1.25s
    assert_eq!(
        super::discard_until_target(decoder.as_mut(), 1_000, 2_250),
        1_250
    );

    let target_sample = 8_000 * 1_250 / 1_000 * 2;
    let next: Vec<f32> = decoder.by_ref().take(4).collect();
    assert_eq!(next, expected[target_sample..target_sample + 4]);
}

#[test]
fn end_seek_finishes_after_discard_exhausts_a_short_fragment() {
    let bytes = [
        include_bytes!("../fixtures/hls-init.mp4").as_slice(),
        include_bytes!("../fixtures/hls-segment.m4s").as_slice(),
    ]
    .concat();
    let duration_hint_ms = 250;
    let source = AudioSource::Bytes(Arc::from(bytes), duration_hint_ms);
    let mut decoder = make_decoder_for_position(&source, 0, false).unwrap();
    assert_eq!(decoder.backend(), crate::decoder::DecoderBackend::Symphonia);

    // 分片实际只有 192ms，平台时长略大时丢弃阶段就会先读到 EOF
    let started = std::time::Instant::now();
    super::discard_until_target(decoder.as_mut(), 0, duration_hint_ms);
    let shared = render_test_shared(1, 8);
    shared.buffering.store(true, Ordering::Release);
    shared.buffer_target_frames.store(
        duration_to_frames(super::REMOTE_PREBUFFER, shared.sample_rate),
        Ordering::Release,
    );
    shared.clock.store_ms(duration_hint_ms);
    let generation = Arc::new(AtomicU64::new(7));
    let cancellation =
        crate::remote::RemoteReadCancellation::new(Arc::clone(&shared.cancelled), None);
    let worker = super::spawn_decode_worker(
        decoder,
        Arc::clone(&shared),
        crate::analyzer::SharedAudioLevel::new(),
        Arc::clone(&generation),
        7,
        cancellation,
    )
    .unwrap();
    let deadline = started + Duration::from_secs(1);
    while !shared.finished.load(Ordering::Acquire) && std::time::Instant::now() < deadline {
        std::thread::yield_now();
    }
    let finished_in_time = shared.finished.load(Ordering::Acquire);
    if !finished_in_time {
        shared.cancelled.store(true, Ordering::Release);
        shared.wake.notify_all();
    }
    worker.join().unwrap();

    assert!(
        finished_in_time,
        "an exhausted seek must finish without waiting for PCM"
    );
    assert!(!shared.buffering.load(Ordering::Acquire));
    assert_eq!(shared.ring.readable_samples(), 0);
    super::wait_until_ready(&shared, &generation, 7, None, &AtomicBool::new(false)).unwrap();
    assert!(started.elapsed() < Duration::from_secs(1));

    let counters = crate::metrics::OutputMetrics::new();
    let mut callback = CallbackState::new(&shared);
    let mut output = [1.0f32; 16];
    callback.render(&shared, &mut output, &counters);
    assert!(output.iter().all(|sample| *sample == 0.0));
    assert_eq!(shared.clock.position_ms(), duration_hint_ms);
    assert_eq!(shared.stretch_buffered.load(Ordering::Acquire), 0);
    assert!(!shared.buffering.load(Ordering::Acquire));
    let snapshot = counters.snapshot();
    assert_eq!((snapshot.underruns, snapshot.rendered_frames), (0, 0));
}

#[test]
fn virtual_body_start_keeps_audio_when_the_segment_start_is_implausible() {
    let (wav, expected) = sine_sections_wav(8_000, &[(0.5, 1)]);
    let source = AudioSource::Bytes(Arc::from(wav), 0);
    let mut decoder = super::make_decoder_for_position(&source, 0, false).unwrap();

    assert_eq!(
        super::discard_until_target(decoder.as_mut(), 0, 60_000),
        0,
        "领先超过 20s 说明落点不对"
    );
    assert_eq!(
        super::discard_until_target(decoder.as_mut(), 5_000, 4_000),
        0,
        "分片晚于目标时没有可丢的"
    );
    assert_eq!(decoder.next(), Some(expected[0]));
}

#[test]
fn loudness_scan_stops_when_cancelled() {
    let (wav, _) = quiet_then_loud_wav(48_000, 2);
    let source = AudioSource::Bytes(Arc::from(wav), 0);

    let result = super::scan_track_loudness(&source, 48_000, 2, &AtomicBool::new(true));

    assert_eq!(result.unwrap_err(), super::LOUDNESS_SCAN_CANCELLED);
}

/// 边下边播要等下载完整才算扫完；下载中断时读到的结尾不是曲尾，不能拿半首当整首
#[test]
fn growing_scan_finishes_only_with_the_whole_download() {
    let (wav, _) = quiet_then_loud_wav(48_000, 1);
    let cancel = Arc::new(AtomicBool::new(false));
    let complete = crate::growing::GrowingAudioBuffer::new();
    complete.append(&wav);
    complete.finish();
    let source =
        super::loudness_scan_source(&AudioSource::growing(complete.reader(), 0), &cancel).unwrap();
    let (_, frames) = super::scan_track_loudness(&source, 48_000, 2, &cancel).unwrap();
    assert_eq!(frames, 96_000);

    let interrupted = crate::growing::GrowingAudioBuffer::new();
    interrupted.append(&wav[..wav.len() / 2]);
    let source =
        super::loudness_scan_source(&AudioSource::growing(interrupted.reader(), 0), &cancel)
            .unwrap();
    let aborter = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(200));
        interrupted.abort();
    });
    let result = super::scan_track_loudness(&source, 48_000, 2, &cancel);
    aborter.join().unwrap();

    assert!(result.is_err(), "下载中断时不能当成整首: {result:?}");
}

/// 释放文件要等这个文件上的扫描线程都退出，别的文件上的扫描不受影响
#[test]
fn releasing_a_file_waits_for_its_loudness_scans_only() {
    let directory = tempfile::tempdir().unwrap();
    let released = directory.path().join("released.flac");
    let kept = directory.path().join("kept.flac");
    std::fs::write(&released, b"audio").unwrap();
    std::fs::write(&kept, b"audio").unwrap();
    let spawn_scan = |path: &Path| {
        let cancel = Arc::new(AtomicBool::new(false));
        let exited = Arc::new(AtomicBool::new(false));
        let (thread_cancel, thread_exited) = (Arc::clone(&cancel), Arc::clone(&exited));
        let handle = std::thread::spawn(move || {
            while !thread_cancel.load(Ordering::Acquire) {
                std::thread::sleep(Duration::from_millis(1));
            }
            std::thread::sleep(Duration::from_millis(20));
            thread_exited.store(true, Ordering::Release);
        });
        super::register_file_scan(&path.to_string_lossy(), Arc::clone(&cancel), handle);
        (cancel, exited)
    };
    let (_, released_exited) = spawn_scan(&released);
    let (kept_cancel, kept_exited) = spawn_scan(&kept);

    super::stop_file_scans(&released);

    assert!(
        released_exited.load(Ordering::Acquire),
        "返回前扫描线程必须已经退出"
    );
    assert!(
        !kept_cancel.load(Ordering::Acquire),
        "别的文件上的扫描不受影响"
    );
    super::stop_file_scans(&kept);
    assert!(kept_exited.load(Ordering::Acquire));
}

#[test]
fn remote_cache_entry_follows_the_committed_session() {
    // 回归：远程流的 current_path 只是 "__remote__"，换缓存目录时认不出它正在读写的缓存文件
    let (mut player, _) = player_without_output(8);
    let started = super::PlaybackStarted {
        duration_ms: 192,
        clock: Arc::new(PlaybackClock::new(0)),
        audio_info: super::SourceAudioInfo::default(),
        cache_entry: Some("ab12".into()),
    };
    player
        .complete_start(started, 8, "__remote__".into())
        .unwrap();
    assert_eq!(player.current_cache_entry.as_deref(), Some("ab12"));
    player.stop();
    assert!(player.current_cache_entry.is_none());
}
