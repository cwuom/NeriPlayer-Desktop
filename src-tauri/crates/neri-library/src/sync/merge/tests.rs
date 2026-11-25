use super::lyrics::{normalize_lyric_state, select_lyric_override};
use super::*;

fn usage_open(track_count: i64) -> crate::library::playlist_usage::PlaylistUsageOpen {
    crate::library::playlist_usage::PlaylistUsageOpen {
        key: "netease:7".into(),
        source: "netease".into(),
        id: 7,
        subtype: None,
        name: "Mix".into(),
        cover_url: Some("https://example.test/cover.jpg".into()),
        track_count,
        fid: 0,
        mid: 0,
        browse_id: None,
        playlist_id: None,
        subtitle: None,
    }
}

#[test]
fn playlist_opens_count_this_device_and_add_up_with_other_devices() {
    let mut extensions = serde_json::Map::new();
    assert!(record_playlist_open(
        &mut extensions,
        &usage_open(10),
        "desktop",
        100
    ));
    assert!(record_playlist_open(
        &mut extensions,
        &usage_open(11),
        "desktop",
        200
    ));
    let entry = extensions["playlistUsageStats"][0].clone();
    assert_eq!(
        [
            json_i64(&entry, "openCount"),
            json_i64(&entry, "firstOpenedAt"),
            json_i64(&entry, "lastOpenedAt"),
            json_i64(&entry, "trackCount")
        ],
        [2, 100, 200, 11]
    );
    assert_eq!(entry["counterShards"][0]["deviceId"], "desktop");
    assert_eq!(entry["counterShards"][0]["playCount"], 2);
    crate::sync::archive::prepare(
        &SyncData {
            extensions: extensions.clone(),
            ..Default::default()
        },
        None,
    )
    .expect("the record must satisfy the strict archive schema");

    let mut android = entry.clone();
    android["counterShards"] = serde_json::json!([{"deviceId": "android", "epochStartedAt": 0, "totalListenMs": 0, "playCount": 3, "firstPlayedAt": 50, "lastPlayedAt": 60}]);
    android["openCount"] = 3.into();
    android["lastOpenedAt"] = 60.into();
    let remote = serde_json::Map::from_iter([(
        "playlistUsageStats".to_string(),
        serde_json::json!([android]),
    )]);
    let merged = merge_extensions(&extensions, &remote);
    assert_eq!(
        json_i64(&merged["playlistUsageStats"][0], "openCount"),
        5,
        "opens on both devices add up"
    );

    assert!(
        record_playlist_open(&mut extensions, &usage_open(0), "desktop", 300),
        "an emptied playlist is removed"
    );
    assert!(extensions["playlistUsageStats"]
        .as_array()
        .unwrap()
        .is_empty());
    assert!(!record_playlist_open(
        &mut extensions,
        &usage_open(0),
        "desktop",
        400
    ));
}

#[test]
fn local_playlist_plays_count_the_total_and_the_day_on_this_device() {
    let day = 86_400_000;
    let mut extensions = serde_json::Map::new();
    assert!(record_local_playlist_play(
        &mut extensions,
        7,
        day,
        "desktop",
        day + 100
    ));
    assert!(record_local_playlist_play(
        &mut extensions,
        7,
        day,
        "desktop",
        day + 200
    ));
    assert!(record_local_playlist_play(
        &mut extensions,
        7,
        2 * day,
        "desktop",
        2 * day + 100
    ));
    assert!(!record_local_playlist_play(
        &mut extensions,
        0,
        day,
        "desktop",
        day
    ));
    let stat = extensions["localPlaylistPlaybackStats"][0].clone();
    assert_eq!(
        [
            json_i64(&stat, "totalPlayCount"),
            json_i64(&stat, "firstPlayedAt"),
            json_i64(&stat, "lastPlayedAt")
        ],
        [3, day + 100, 2 * day + 100]
    );
    let buckets: Vec<(i64, i64)> = extensions["localPlaylistPlaybackBuckets"]
        .as_array()
        .unwrap()
        .iter()
        .map(|bucket| {
            (
                json_i64(bucket, "dayStartAt"),
                json_i64(bucket, "playCount"),
            )
        })
        .collect();
    assert_eq!(buckets, [(day, 2), (2 * day, 1)]);
    crate::sync::archive::prepare(
        &SyncData {
            extensions: extensions.clone(),
            ..Default::default()
        },
        None,
    )
    .expect("the records must satisfy the strict archive schema");

    let mut android = stat.clone();
    android["counterShards"] = serde_json::json!([{"deviceId": "android", "epochStartedAt": 0, "totalListenMs": 0, "playCount": 4, "firstPlayedAt": 10, "lastPlayedAt": 20}]);
    android["totalPlayCount"] = 4.into();
    let remote = serde_json::Map::from_iter([(
        "localPlaylistPlaybackStats".to_string(),
        serde_json::json!([android]),
    )]);
    let merged = merge_extensions(&extensions, &remote);
    assert_eq!(
        json_i64(&merged["localPlaylistPlaybackStats"][0], "totalPlayCount"),
        7,
        "plays on both devices add up"
    );
}

#[test]
fn reopening_a_playlist_removed_elsewhere_starts_a_record_that_survives_the_merge() {
    let mut extensions = serde_json::Map::from_iter([
        (
            "playlistUsageDeletions".to_string(),
            serde_json::json!([{"playlistKey": "netease:7", "deletionTokens": [{"deviceId": "android", "counter": 4}], "deletedAt": 90}]),
        ),
        (
            "playlistUsageStats".to_string(),
            serde_json::json!([{"playlistKey": "netease:7", "source": "netease", "id": 7, "name": "Old", "trackCount": 5, "firstOpenedAt": 10, "lastOpenedAt": 20, "openCount": 9, "counterBaseOpenCount": 9, "counterShards": [], "fid": 0, "mid": 0, "observedDeletionTokens": []}]),
        ),
    ]);
    assert!(record_playlist_open(
        &mut extensions,
        &usage_open(10),
        "desktop",
        100
    ));
    let entry = &extensions["playlistUsageStats"][0];
    assert_eq!(
        json_i64(entry, "openCount"),
        1,
        "counts from before the removal are dropped"
    );
    assert_eq!(
        entry["observedDeletionTokens"],
        serde_json::json!([{"deviceId": "android", "counter": 4}])
    );
    let merged = merge_extensions(&extensions, &serde_json::Map::new());
    assert_eq!(merged["playlistUsageStats"].as_array().unwrap().len(), 1);
}

#[test]
fn lyric_versions_reset_and_utf16_ties_match_android_independent_of_input_order() {
    let unknown = SyncSong {
        id: "1".into(),
        lyric_sync_revision: 5,
        ..Default::default()
    };
    assert_eq!(normalize_lyric_state(&unknown).lyric_sync_revision, 0);
    let edit = SyncSong {
        id: "1".into(),
        lyric_sync_edited: Some(true),
        lyric_sync_revision: 7,
        matched_lyric: Some("edit".into()),
        ..Default::default()
    };
    let reset = SyncSong {
        id: "1".into(),
        lyric_sync_edited: Some(false),
        lyric_sync_revision: 7,
        ..Default::default()
    };
    let supplementary = SyncSong {
        matched_lyric: Some("\u{10000}".into()),
        ..edit.clone()
    };
    let bmp = SyncSong {
        matched_lyric: Some("\u{e000}\u{e000}".into()),
        ..edit.clone()
    };
    for (first, second) in [
        (supplementary.clone(), bmp.clone()),
        (bmp.clone(), supplementary.clone()),
    ] {
        let mut registry = HashMap::new();
        select_lyric_override(&mut registry, first);
        select_lyric_override(&mut registry, second);
        assert_eq!(
            registry.values().next().unwrap().matched_lyric,
            bmp.matched_lyric
        );
    }
    for songs in [vec![edit.clone(), reset.clone()], vec![reset, edit]] {
        let mut registry = HashMap::new();
        for song in songs {
            select_lyric_override(&mut registry, normalize_lyric_state(&song));
        }
        assert_eq!(
            registry.values().next().unwrap().lyric_sync_edited,
            Some(false)
        );
        assert!(registry.values().next().unwrap().matched_lyric.is_none());
    }
}

#[test]
fn latest_lyric_survives_older_selected_display_metadata_and_registry_stays_minimal() {
    let mut local = sync_data(vec![playlist(vec![SyncSong {
        id: "1".into(),
        name: "desktop title".into(),
        added_at: 20,
        sync_metadata_version: 1,
        lyric_sync_edited: Some(true),
        lyric_sync_revision: 2,
        matched_lyric: Some("old".into()),
        ..Default::default()
    }])]);
    let mut remote = local.clone();
    local.playlists[0].modified_at = 30;
    remote.playlists[0].modified_at = 10;
    remote.playlists[0].songs[0].lyric_sync_revision = 3;
    remote.playlists[0].songs[0].matched_lyric = Some("new\n".into());
    let merged = three_way_merge(&local, &remote, 0, &HashMap::new());
    assert_eq!(
        merged.playlists[0].songs[0].matched_lyric.as_deref(),
        Some("new\n")
    );
    assert_eq!(merged.extensions["lyricOverrides"][0]["name"], "");
    assert_eq!(merged.extensions["lyricOverrides"][0]["addedAt"], 0);
}

#[test]
fn legacy_bilibili_lyric_reference_restores_the_committed_override() {
    let committed = SyncSong {
        id: "1".into(),
        album: "Bilibili|123|BVtest".into(),
        channel_id: Some("bilibili".into()),
        audio_id: Some("BVtest".into()),
        lyric_sync_edited: Some(true),
        lyric_sync_revision: 10,
        matched_lyric: Some("committed\n".into()),
        original_lyric: Some("original\n".into()),
        ..Default::default()
    };
    let reference = SyncSong {
        sub_audio_id: Some("123".into()),
        matched_lyric: None,
        original_lyric: None,
        lyric_sync_revision: 9,
        ..committed.clone()
    };
    let mut data = sync_data(vec![playlist(vec![reference.clone()])]);
    data.extensions
        .insert("lyricOverrides".into(), serde_json::json!([committed]));
    converge_lyrics(&mut data, true).unwrap();
    let restored = &data.playlists[0].songs[0];
    assert_eq!(restored.matched_lyric.as_deref(), Some("committed\n"));
    assert_eq!(restored.original_lyric.as_deref(), Some("original\n"));
    assert_eq!(restored.lyric_sync_revision, 10);
    assert_eq!(
        data.extensions["lyricOverrides"].as_array().unwrap().len(),
        1
    );

    for invalid in [
        Vec::new(),
        vec![SyncSong {
            sub_audio_id: Some("124".into()),
            ..committed.clone()
        }],
        vec![SyncSong {
            lyric_sync_revision: 8,
            ..committed.clone()
        }],
    ] {
        let mut data = sync_data(vec![playlist(vec![reference.clone()])]);
        data.extensions
            .insert("lyricOverrides".into(), serde_json::json!(invalid));
        let error = converge_lyrics(&mut data, true).unwrap_err();
        assert!(error
            .to_string()
            .contains("Sync lyric reference has no committed override"));
    }
}

#[test]
fn bili_skip_merge_applies_android_interval_limit_before_merging() {
    let intervals: Vec<_> = (0..102)
        .map(|start| serde_json::json!({"startMs": start, "endMs": start + 1}))
        .collect();
    let local = serde_json::Map::from_iter([(
        "biliVideoSkipRules".into(),
        serde_json::json!([{
            "bvid": "BVtest", "cid": 9, "intervals": intervals, "modifiedAt": 10, "isDeleted": false
        }]),
    )]);
    let merged = merge_extensions(&local, &serde_json::Map::new());
    assert_eq!(
        merged["biliVideoSkipRules"][0]["intervals"],
        serde_json::json!([{"startMs": 0, "endMs": 100}])
    );
}

#[test]
fn bili_skip_rules_converge_in_both_directions_and_tombstones_survive() {
    let active = serde_json::json!({"bvid":"BVtest","cid":9,"intervals":[{"startMs":10,"endMs":20}],"modifiedAt":10,"isDeleted":false});
    let mut adjacent = active.clone();
    adjacent["intervals"] = serde_json::json!([{"startMs":20,"endMs":30}]);
    let deleted = serde_json::json!({"bvid":"BVtest","cid":10,"intervals":[],"modifiedAt":11,"isDeleted":true});
    let local = serde_json::Map::from_iter([(
        "biliVideoSkipRules".into(),
        serde_json::json!([active, deleted.clone()]),
    )]);
    let remote = serde_json::Map::from_iter([(
        "biliVideoSkipRules".into(),
        serde_json::json!([adjacent, {"bvid":"BVtest","cid":10,"intervals":[{"startMs":0,"endMs":1}],"modifiedAt":10,"isDeleted":false}]),
    )]);
    let merged = merge_extensions(&local, &remote);
    assert_eq!(merged, merge_extensions(&remote, &local));
    assert_eq!(
        merged["biliVideoSkipRules"][0]["intervals"],
        serde_json::json!([{"startMs":10,"endMs":30}])
    );
    assert_eq!(merged["biliVideoSkipRules"][1], deleted);
    assert_eq!(merge_extensions(&merged, &merged), merged);
}

#[test]
fn usage_deletion_proofs_are_checked_per_candidate_before_union() {
    let deletion = serde_json::json!({"playlistKey":"key","deletionTokens":[{"deviceId":"a","counter":1},{"deviceId":"b","counter":1}],"deletedAt":10});
    let left = serde_json::json!({"playlistKey":"key","openCount":1,"observedDeletionTokens":[{"deviceId":"a","counter":1}]});
    let right = serde_json::json!({"playlistKey":"key","openCount":2,"observedDeletionTokens":[{"deviceId":"b","counter":1}]});
    let local = serde_json::Map::from_iter([
        ("playlistUsageStats".into(), serde_json::json!([left])),
        (
            "playlistUsageDeletions".into(),
            serde_json::json!([deletion]),
        ),
    ]);
    let remote =
        serde_json::Map::from_iter([("playlistUsageStats".into(), serde_json::json!([right]))]);
    assert!(merge_extensions(&local, &remote)["playlistUsageStats"]
        .as_array()
        .unwrap()
        .is_empty());
}

#[test]
fn local_playlist_aggregate_is_lifted_to_all_day_buckets_without_losing_shards() {
    let local = serde_json::Map::from_iter([(
        "localPlaylistPlaybackStats".into(),
        serde_json::json!([{"playlistId":1,"totalPlayCount":1,"firstPlayedAt":30,"lastPlayedAt":40,"counterShards":[{"deviceId":"phone","epochStartedAt":1,"playCount":1,"firstPlayedAt":30,"lastPlayedAt":40}]}]),
    )]);
    let remote = serde_json::Map::from_iter([(
        "localPlaylistPlaybackBuckets".into(),
        serde_json::json!([{"playlistId":1,"dayStartAt":1,"playCount":3,"firstPlayedAt":10,"lastPlayedAt":20},{"playlistId":1,"dayStartAt":2,"playCount":4,"firstPlayedAt":50,"lastPlayedAt":60},{"playlistId":2,"dayStartAt":1,"playCount":5,"firstPlayedAt":2,"lastPlayedAt":4}]),
    )]);
    for merged in [
        merge_extensions(&local, &remote),
        merge_extensions(&remote, &local),
    ] {
        assert_eq!(merged["localPlaylistPlaybackStats"][0]["totalPlayCount"], 7);
        assert_eq!(merged["localPlaylistPlaybackStats"][0]["firstPlayedAt"], 10);
        assert_eq!(merged["localPlaylistPlaybackStats"][0]["lastPlayedAt"], 60);
        assert_eq!(
            merged["localPlaylistPlaybackStats"][0]["counterShards"][0]["deviceId"],
            "phone"
        );
        assert_eq!(merged["localPlaylistPlaybackStats"][1]["playlistId"], 2);
        assert_eq!(merged["localPlaylistPlaybackStats"][1]["totalPlayCount"], 5);
    }
}

#[test]
fn remote_phone_reorder_keeps_remote_added_at_values_and_order() {
    let mut local = playlist(vec![song("1", 300), song("2", 200), song("3", 100)]);
    local.modified_at = 100;
    let mut remote = playlist(vec![song("3", 900), song("1", 899), song("2", 898)]);
    remote.modified_at = 300;

    let merged = three_way_merge(
        &sync_data(vec![local]),
        &sync_data(vec![remote]),
        200,
        &HashMap::new(),
    );

    assert_eq!(
        merged.playlists[0]
            .songs
            .iter()
            .map(|song| song.id.as_str())
            .collect::<Vec<_>>(),
        vec!["3", "1", "2"]
    );
    assert_eq!(
        merged.playlists[0]
            .songs
            .iter()
            .map(|song| song.added_at)
            .collect::<Vec<_>>(),
        vec![900, 899, 898]
    );
}

#[test]
fn local_desktop_reorder_keeps_local_added_at_values_and_order() {
    let mut local = playlist(vec![song("2", 900), song("3", 899), song("1", 898)]);
    local.modified_at = 300;
    let mut remote = playlist(vec![song("1", 300), song("2", 200), song("3", 100)]);
    remote.modified_at = 100;

    let merged = three_way_merge(
        &sync_data(vec![local]),
        &sync_data(vec![remote]),
        200,
        &HashMap::new(),
    );

    assert_eq!(
        merged.playlists[0]
            .songs
            .iter()
            .map(|song| song.id.as_str())
            .collect::<Vec<_>>(),
        vec!["2", "3", "1"]
    );
    assert_eq!(
        merged.playlists[0]
            .songs
            .iter()
            .map(|song| song.added_at)
            .collect::<Vec<_>>(),
        vec![900, 899, 898]
    );
}

#[test]
fn missing_created_at_does_not_replace_valid_timestamp() {
    let mut local = playlist(Vec::new());
    local.created_at = 100;
    let mut remote = local.clone();
    remote.created_at = 0;

    let merged = three_way_merge(
        &sync_data(vec![local]),
        &sync_data(vec![remote]),
        0,
        &HashMap::new(),
    );

    assert_eq!(merged.playlists[0].created_at, 100);
}

#[test]
fn merge_preserves_remote_added_at_when_desktop_local_lacks_it() {
    let local_song = song("42", 0);
    let remote_song = song("42", 500);
    let base_key = remote_song.identity().stable_key();
    let base_snapshot = HashMap::from([("1".to_string(), HashSet::from([base_key]))]);
    let merged = three_way_merge(
        &sync_data(vec![playlist(vec![local_song])]),
        &sync_data(vec![playlist(vec![remote_song])]),
        100,
        &base_snapshot,
    );
    assert_eq!(merged.playlists[0].songs[0].added_at, 500);
}

#[test]
fn local_deleted_song_is_not_resurrected_from_remote() {
    let deleted_song = song("42", 500);
    let base_snapshot = HashMap::from([(
        "1".to_string(),
        HashSet::from([deleted_song.identity().stable_key()]),
    )]);
    let merged = three_way_merge(
        &sync_data(vec![playlist(Vec::new())]),
        &sync_data(vec![playlist(vec![deleted_song])]),
        100,
        &base_snapshot,
    );
    assert!(merged.playlists[0].songs.is_empty());
}

#[test]
fn remote_deleted_song_is_not_resurrected_from_local() {
    let deleted_song = song("42", 500);
    let base_snapshot = HashMap::from([(
        "1".to_string(),
        HashSet::from([deleted_song.identity().stable_key()]),
    )]);
    let merged = three_way_merge(
        &sync_data(vec![playlist(vec![deleted_song])]),
        &sync_data(vec![playlist(Vec::new())]),
        100,
        &base_snapshot,
    );
    assert!(merged.playlists[0].songs.is_empty());
}

#[test]
fn playlist_deletion_tombstone_is_preserved() {
    let active = playlist(Vec::new());
    let mut deleted = active.clone();
    deleted.is_deleted = true;
    deleted.modified_at = 200;
    let merged = three_way_merge(
        &sync_data(vec![active]),
        &sync_data(vec![deleted]),
        100,
        &HashMap::new(),
    );
    assert!(merged.playlists[0].is_deleted);
    assert!(merged.playlists[0].songs.is_empty());
}

fn edited_lyrics(revision: i64, text: &str) -> SyncSong {
    SyncSong {
        lyric_sync_edited: Some(true),
        lyric_sync_revision: revision,
        matched_lyric: Some(text.into()),
        ..song("1", 10)
    }
}

#[test]
fn recorded_lyric_overrides_keep_the_highest_revision() {
    let mut extensions = serde_json::Map::new();
    record_lyric_override(&mut extensions, &edited_lyrics(10, "first")).unwrap();
    record_lyric_override(&mut extensions, &edited_lyrics(5, "older")).unwrap();
    assert_eq!(extensions["lyricOverrides"][0]["matchedLyric"], "first");
    record_lyric_override(&mut extensions, &edited_lyrics(20, "newer")).unwrap();
    assert_eq!(extensions["lyricOverrides"].as_array().unwrap().len(), 1);
    assert_eq!(extensions["lyricOverrides"][0]["matchedLyric"], "newer");
}

#[test]
fn a_desktop_lyric_edit_after_the_phone_edit_wins_the_merge() {
    let phone = edited_lyrics(5, "phone");
    let desk = edited_lyrics(1_800_000_000_000, "desk");
    for (local, remote) in [(&desk, &phone), (&phone, &desk)] {
        let merged = three_way_merge(
            &sync_data(vec![playlist(vec![local.clone()])]),
            &sync_data(vec![playlist(vec![remote.clone()])]),
            0,
            &HashMap::new(),
        );
        assert_eq!(
            merged.playlists[0].songs[0].matched_lyric.as_deref(),
            Some("desk")
        );
    }
}

#[test]
fn a_playlist_edited_after_its_deletion_survives_the_merge() {
    let mut tombstone = playlist(Vec::new());
    tombstone.is_deleted = true;
    tombstone.modified_at = 100;
    let mut edited = playlist(vec![song("42", 150)]);
    edited.modified_at = 200;
    for (local, remote) in [(&tombstone, &edited), (&edited, &tombstone)] {
        let merged = three_way_merge(
            &sync_data(vec![local.clone()]),
            &sync_data(vec![remote.clone()]),
            50,
            &HashMap::new(),
        );
        assert!(
            !merged.playlists[0].is_deleted,
            "the newer live copy wins on either side"
        );
        assert_eq!(merged.playlists[0].songs.len(), 1);
    }
    edited.modified_at = 100;
    let merged = three_way_merge(
        &sync_data(vec![tombstone]),
        &sync_data(vec![edited]),
        50,
        &HashMap::new(),
    );
    assert!(
        merged.playlists[0].is_deleted,
        "a deletion at the same time as the last edit is kept"
    );
}

#[test]
fn playlist_song_deletion_is_applied() {
    let target = song("42", 100);
    let deletion = SyncPlaylistSongDeletion {
        playlist_id: "1".into(),
        song_id: target.id.clone(),
        album: target.album.clone(),
        deleted_at: 200,
        device_id: "desktop".into(),
        ..Default::default()
    };
    let mut local = sync_data(vec![playlist(vec![target])]);
    local.playlist_song_deletions = vec![deletion];
    let merged = three_way_merge(&local, &SyncData::default(), 0, &HashMap::new());
    assert!(merged.playlists[0].songs.is_empty());
}

#[test]
fn legacy_added_at_prevents_display_order_migration_from_reviving_deleted_song() {
    let mut target = song("42", 900);
    target.legacy_added_at = Some(100);
    let deletion = SyncPlaylistSongDeletion {
        playlist_id: "1".into(),
        song_id: target.id.clone(),
        album: target.album.clone(),
        deleted_at: 200,
        device_id: "android".into(),
        ..Default::default()
    };

    let remaining = apply_playlist_song_deletions("1", &[target], &[deletion]);

    assert!(remaining.is_empty());
}

#[test]
fn local_primary_does_not_clear_remote_lyrics() {
    let mut local = song("42", 500);
    local.matched_lyric = Some(String::new());
    local.sync_metadata_version = LEGACY_SYNC_METADATA_VERSION;
    let mut remote = song("42", 500);
    remote.matched_lyric = Some("[00:01.00]cloud lyric".into());
    remote.matched_translated_lyric = Some("[00:01.00]translation".into());
    remote.original_lyric = Some("original cloud lyric".into());
    remote.sync_metadata_version = LEGACY_SYNC_METADATA_VERSION;

    let merged = merge_songs(
        &[local],
        &[remote],
        300,
        200,
        true,
        true,
        100,
        false,
        &HashSet::new(),
        "1",
        &[],
    );

    assert_eq!(
        merged[0].matched_lyric.as_deref(),
        Some("[00:01.00]cloud lyric")
    );
    assert_eq!(
        merged[0].matched_translated_lyric.as_deref(),
        Some("[00:01.00]translation")
    );
    assert_eq!(
        merged[0].original_lyric.as_deref(),
        Some("original cloud lyric")
    );
    assert_eq!(
        merged[0].sync_metadata_version,
        CURRENT_SYNC_METADATA_VERSION
    );
}

#[test]
fn legacy_primary_order_keeps_current_rich_metadata() {
    let mut current = song("42", 100);
    current.matched_lyric = Some("[00:01.00]lyric".into());
    current.custom_name = Some("Custom title".into());
    current.original_name = Some("Original".into());
    let mut legacy_primary = current.clone();
    legacy_primary.name = "Custom title".into();
    legacy_primary.added_at = 900;
    legacy_primary.matched_lyric = None;
    legacy_primary.custom_name = None;
    legacy_primary.original_name = None;
    legacy_primary.sync_metadata_version = LEGACY_SYNC_METADATA_VERSION;

    let merged = merge_songs(
        &[current],
        &[legacy_primary],
        100,
        300,
        false,
        true,
        200,
        false,
        &HashSet::new(),
        "1",
        &[],
    );

    assert_eq!(merged[0].added_at, 900);
    assert_eq!(merged[0].name, "Song");
    assert_eq!(merged[0].matched_lyric.as_deref(), Some("[00:01.00]lyric"));
    assert_eq!(merged[0].custom_name.as_deref(), Some("Custom title"));
    assert_eq!(
        merged[0].sync_metadata_version,
        CURRENT_SYNC_METADATA_VERSION
    );
}

#[test]
fn current_primary_can_intentionally_clear_metadata() {
    let mut local = song("42", 100);
    local.matched_lyric = Some("[00:01.00]old lyric".into());
    local.custom_name = Some("Old custom title".into());
    let mut remote = local.clone();
    remote.added_at = 200;
    remote.matched_lyric = None;
    remote.custom_name = None;

    let merged = merge_songs(
        &[local],
        &[remote],
        100,
        300,
        false,
        true,
        200,
        false,
        &HashSet::new(),
        "1",
        &[],
    );

    assert_eq!(merged[0].added_at, 200);
    assert!(merged[0].matched_lyric.is_none());
    assert!(merged[0].custom_name.is_none());
}

#[test]
fn remote_only_change_uses_remote_membership_and_payload() {
    let local_token = SyncCausalToken {
        device_id: "desktop".into(),
        counter: 1,
    };
    let remote_token = SyncCausalToken {
        device_id: "android".into(),
        counter: 1,
    };
    let mut local_match = song("42", 100);
    local_match.name = "Local metadata".into();
    local_match.sync_membership_tokens = vec![local_token.clone()];
    let mut remote_match = local_match.clone();
    remote_match.name = "Remote metadata".into();
    remote_match.sync_membership_tokens = vec![remote_token.clone()];
    let local_only = song("99", 100);

    let merged = merge_songs(
        &[local_match, local_only],
        &[remote_match],
        100,
        200,
        false,
        true,
        150,
        false,
        &HashSet::new(),
        "1",
        &[],
    );

    assert_eq!(merged.len(), 1);
    assert_eq!(merged[0].name, "Remote metadata");
    assert_eq!(
        merged[0].sync_membership_tokens,
        vec![remote_token, local_token]
    );
}

#[test]
fn bridge_aliases_collapse_the_full_membership_component() {
    let token_a = SyncCausalToken {
        device_id: "a".into(),
        counter: 1,
    };
    let token_b = SyncCausalToken {
        device_id: "b".into(),
        counter: 1,
    };
    let mut first = song("1", 100);
    first.sync_membership_tokens = vec![token_a.clone()];
    let mut second = song("2", 100);
    second.sync_membership_tokens = vec![token_b.clone()];
    let mut bridge = first.clone();
    bridge.sync_membership_tokens = vec![token_b.clone()];

    let merged = deduplicate_songs(&[first, second, bridge]);

    assert_eq!(merged.len(), 1);
    assert_eq!(merged[0].sync_membership_tokens, vec![token_a, token_b]);
}

#[test]
fn counter_shards_keep_multiple_epochs_in_stable_order() {
    let left = SyncPlaybackCounterShard {
        device_id: "d".into(),
        epoch_started_at: 1,
        play_count: 1,
        ..Default::default()
    };
    let right = SyncPlaybackCounterShard {
        device_id: "d".into(),
        epoch_started_at: 2,
        play_count: 2,
        ..Default::default()
    };
    let merged = merge_counter_shards(&[left], &[right]);
    assert_eq!(
        merged
            .iter()
            .map(|shard| shard.epoch_started_at)
            .collect::<Vec<_>>(),
        vec![1, 2]
    );
}

#[test]
fn data_change_detection_includes_song_metadata_and_deletions() {
    let remote = sync_data(vec![playlist(vec![song("42", 10_000)])]);
    let mut merged = remote.clone();
    merged.playlists[0].songs[0].name = "Changed".into();
    assert!(has_data_changed(&remote, &merged));
}

#[test]
fn playlist_timestamp_only_change_is_not_data_change() {
    let remote = sync_data(vec![playlist(vec![song("42", 10_000)])]);
    let mut merged = remote.clone();
    merged.playlists[0].created_at = 999;
    merged.playlists[0].modified_at = 1_000_000;
    assert!(!has_data_changed(&remote, &merged));
}

#[test]
fn favorite_display_only_fields_are_not_data_change() {
    let remote = SyncData {
        favorite_playlists: vec![SyncFavoritePlaylist {
            id: "7".into(),
            name: "Fav".into(),
            cover_url: "https://a".into(),
            track_count: 1,
            source: "netease".into(),
            songs: vec![song("42", 10)],
            added_time: 1,
            modified_at: 2,
            is_deleted: false,
            sort_order: 3,
            browse_id: Some("B".into()),
            playlist_id: Some("P".into()),
            subtitle: Some("old".into()),
        }],
        ..Default::default()
    };
    let mut merged = remote.clone();
    // 仅展示字段变化不应触发回声上传
    merged.favorite_playlists[0].name = "Fav Renamed".into();
    merged.favorite_playlists[0].cover_url = "https://b".into();
    merged.favorite_playlists[0].added_time = 99;
    merged.favorite_playlists[0].browse_id = Some("B2".into());
    merged.favorite_playlists[0].subtitle = Some("new".into());
    assert!(!has_data_changed(&remote, &merged));

    // 内容字段变化仍应检测
    merged.favorite_playlists[0].sort_order = 4;
    assert!(has_data_changed(&remote, &merged));
}

#[test]
fn favorite_deletion_ties_win_in_both_merge_directions() {
    let deleted = SyncFavoritePlaylist {
        id: "7".into(),
        name: "Deleted favorite".into(),
        cover_url: String::new(),
        source: "netease".into(),
        songs: vec![song("1", 10)],
        track_count: 1,
        added_time: 40,
        modified_at: 100,
        is_deleted: true,
        sort_order: 9,
        browse_id: None,
        playlist_id: None,
        subtitle: None,
    };
    let active = SyncFavoritePlaylist {
        name: "Active favorite".into(),
        songs: vec![song("2", 10)],
        added_time: 80,
        is_deleted: false,
        sort_order: 4,
        ..deleted.clone()
    };

    for (left, right) in [(&active, &deleted), (&deleted, &active)] {
        let merged =
            merge_favorite_playlists(std::slice::from_ref(left), std::slice::from_ref(right));
        assert_eq!(merged.len(), 1);
        let favorite = &merged[0];
        assert!(favorite.is_deleted);
        assert_eq!(favorite.name, "Deleted favorite");
        assert!(favorite.songs.is_empty());
        assert_eq!(favorite.track_count, 0);
        assert_eq!(favorite.modified_at, 100);
        assert_eq!(favorite.added_time, 80);
        assert_eq!(favorite.sort_order, 9);
    }
}

#[test]
fn favorite_restore_after_deletion_is_retained() {
    let deleted = SyncFavoritePlaylist {
        id: "7".into(),
        name: String::new(),
        cover_url: String::new(),
        source: "netease".into(),
        songs: Vec::new(),
        track_count: 0,
        added_time: 0,
        modified_at: 100,
        is_deleted: true,
        sort_order: 9,
        browse_id: None,
        playlist_id: None,
        subtitle: None,
    };
    let restored = SyncFavoritePlaylist {
        modified_at: 110,
        is_deleted: false,
        songs: vec![song("2", 110)],
        sort_order: 4,
        ..deleted.clone()
    };
    for (left, right) in [(&restored, &deleted), (&deleted, &restored)] {
        let merged =
            merge_favorite_playlists(std::slice::from_ref(left), std::slice::from_ref(right));
        assert_eq!(merged.len(), 1);
        assert!(!merged[0].is_deleted);
        assert_eq!(merged[0].songs.len(), 1);
        assert_eq!(merged[0].songs[0].id, "2");
        assert_eq!(merged[0].track_count, 1);
        assert_eq!(merged[0].sort_order, 4);
    }
}

#[test]
fn playback_clear_rejects_old_epoch_despite_a_new_last_play() {
    let shard = SyncPlaybackCounterShard {
        device_id: "android".into(),
        epoch_started_at: 0,
        first_played_at: 50,
        last_played_at: 110,
        total_listen_ms: 1_000,
        play_count: 10,
        ..Default::default()
    };
    let stat = SyncTrackStat {
        identity_key: "k".into(),
        first_played_at: 50,
        last_played_at: 110,
        total_listen_ms: 1_000,
        play_count: 10,
        counter_shards: vec![shard],
        ..Default::default()
    };
    let bucket = bucket_from_clear_stat(&stat);

    assert!(merge_playback_stats(std::slice::from_ref(&stat), &[], 100).is_empty());
    assert!(merge_playback_stats(&[], std::slice::from_ref(&stat), 100).is_empty());
    assert!(merge_stat_buckets(std::slice::from_ref(&bucket), &[], 100).is_empty());
    assert!(merge_stat_buckets(&[], std::slice::from_ref(&bucket), 100).is_empty());
}

#[test]
fn playback_clear_recomputes_totals_from_only_current_epoch_shards() {
    let shards = vec![
        SyncPlaybackCounterShard {
            device_id: "old".into(),
            epoch_started_at: 0,
            first_played_at: 50,
            last_played_at: 150,
            total_listen_ms: 1_000,
            play_count: 10,
            ..Default::default()
        },
        SyncPlaybackCounterShard {
            device_id: "cross-clear".into(),
            epoch_started_at: 100,
            first_played_at: 90,
            last_played_at: 145,
            total_listen_ms: 20,
            play_count: 2,
            ..Default::default()
        },
        SyncPlaybackCounterShard {
            device_id: "current-a".into(),
            epoch_started_at: 100,
            first_played_at: 100,
            last_played_at: 130,
            total_listen_ms: 300,
            play_count: 3,
            ..Default::default()
        },
        SyncPlaybackCounterShard {
            device_id: "current-b".into(),
            epoch_started_at: 110,
            first_played_at: 115,
            last_played_at: 140,
            total_listen_ms: 400,
            play_count: 4,
            ..Default::default()
        },
    ];
    let stat = SyncTrackStat {
        identity_key: "k".into(),
        first_played_at: 50,
        last_played_at: 150,
        total_listen_ms: 2_620,
        play_count: 28,
        counter_base_listen_ms: 900,
        counter_base_play_count: 9,
        counter_shards: shards,
        ..Default::default()
    };
    let bucket = bucket_from_clear_stat(&stat);
    let merged = merge_playback_stats(
        std::slice::from_ref(&stat),
        std::slice::from_ref(&stat),
        100,
    );
    let buckets = merge_stat_buckets(
        std::slice::from_ref(&bucket),
        std::slice::from_ref(&bucket),
        100,
    );
    assert_eq!(merged.len(), 1);
    assert_eq!(buckets.len(), 1);
    for (total, count, first, last, base_total, base_count, retained) in [
        (
            merged[0].total_listen_ms,
            merged[0].play_count,
            merged[0].first_played_at,
            merged[0].last_played_at,
            merged[0].counter_base_listen_ms,
            merged[0].counter_base_play_count,
            &merged[0].counter_shards,
        ),
        (
            buckets[0].total_listen_ms,
            buckets[0].play_count,
            buckets[0].first_played_at,
            buckets[0].last_played_at,
            buckets[0].counter_base_listen_ms,
            buckets[0].counter_base_play_count,
            &buckets[0].counter_shards,
        ),
    ] {
        assert_eq!((total, count, first, last), (700, 7, 100, 140));
        assert_eq!((base_total, base_count), (0, 0));
        assert_eq!(retained.len(), 2);
        assert_eq!(retained[0].device_id, "current-a");
        assert_eq!(retained[1].device_id, "current-b");
    }
    let again = merge_playback_stats(&merged, &merged, 100);
    let buckets_again = merge_stat_buckets(&buckets, &buckets, 100);
    assert_eq!((again[0].total_listen_ms, again[0].play_count), (700, 7));
    assert_eq!(again[0].counter_shards.len(), 2);
    assert_eq!(
        (
            buckets_again[0].total_listen_ms,
            buckets_again[0].play_count
        ),
        (700, 7)
    );
    assert_eq!(buckets_again[0].counter_shards.len(), 2);
}

#[test]
fn playback_clear_accepts_legacy_counters_only_when_all_plays_follow_clear() {
    for (first, last, retained) in [
        (50, 110, false),
        (100, 110, true),
        (110, 110, true),
        (100, 99, false),
    ] {
        let stat = SyncTrackStat {
            identity_key: "legacy".into(),
            first_played_at: first,
            last_played_at: last,
            total_listen_ms: 600,
            play_count: 6,
            ..Default::default()
        };
        let bucket = bucket_from_clear_stat(&stat);
        let merged = merge_playback_stats(std::slice::from_ref(&stat), &[], 100);
        let buckets = merge_stat_buckets(std::slice::from_ref(&bucket), &[], 100);
        assert_eq!(!merged.is_empty(), retained, "first={first}, last={last}");
        assert_eq!(!buckets.is_empty(), retained, "first={first}, last={last}");
        if retained {
            assert_eq!((merged[0].total_listen_ms, merged[0].play_count), (600, 6));
            assert_eq!(
                (buckets[0].total_listen_ms, buckets[0].play_count),
                (600, 6)
            );
        }
    }
}

fn bucket_from_clear_stat(stat: &SyncTrackStat) -> SyncPlaybackStatBucket {
    SyncPlaybackStatBucket {
        day_start_at: 0,
        identity_key: stat.identity_key.clone(),
        total_listen_ms: stat.total_listen_ms,
        play_count: stat.play_count,
        first_played_at: stat.first_played_at,
        last_played_at: stat.last_played_at,
        counter_base_listen_ms: stat.counter_base_listen_ms,
        counter_base_play_count: stat.counter_base_play_count,
        counter_shards: stat.counter_shards.clone(),
        ..Default::default()
    }
}

#[test]
fn playlist_reordering_alone_is_not_a_data_change() {
    let first = playlist(vec![song("1", 10)]);
    let second = SyncPlaylist {
        id: "2".into(),
        ..playlist(vec![song("2", 20)])
    };
    let remote = sync_data(vec![first.clone(), second.clone()]);
    let reordered = sync_data(vec![second, first]);

    assert!(!has_data_changed(&remote, &reordered));
}

#[test]
fn aggregate_stats_never_fall_below_daily_bucket_totals() {
    let stat = SyncTrackStat {
        identity_key: "k".into(),
        total_listen_ms: 900,
        play_count: 876,
        last_played_at: 100,
        ..Default::default()
    };
    let buckets = vec![
        SyncPlaybackStatBucket {
            day_start_at: 0,
            identity_key: "k".into(),
            total_listen_ms: 600,
            play_count: 500,
            last_played_at: 50,
            ..Default::default()
        },
        SyncPlaybackStatBucket {
            day_start_at: MILLIS_PER_DAY,
            identity_key: "k".into(),
            total_listen_ms: 700,
            play_count: 381,
            last_played_at: 100,
            ..Default::default()
        },
    ];

    let lifted = lift_stats_to_bucket_totals(&[stat], &buckets);

    assert_eq!(lifted[0].play_count, 881);
    assert_eq!(lifted[0].total_listen_ms, 1_300);
    // 单调抬升必须幂等, 否则与对端的 max 合并会来回震荡
    let again = lift_stats_to_bucket_totals(&lifted, &buckets);
    assert_eq!(again[0].play_count, 881);
    assert_eq!(again[0].total_listen_ms, 1_300);
}

#[test]
fn archive_merge_preserves_old_buckets_and_all_stats() {
    let mut local = sync_data(vec![]);
    local.playback_stats = (0..2010)
        .map(|index| SyncTrackStat {
            identity_key: format!("key{index}"),
            last_played_at: index,
            ..Default::default()
        })
        .collect();
    local.playback_stat_buckets = vec![
        stat_bucket(MILLIS_PER_DAY, "old"),
        stat_bucket(1000 * MILLIS_PER_DAY, "new"),
    ];
    let merged = three_way_merge(&local, &SyncData::default(), 0, &HashMap::new());
    assert_eq!(merged.playback_stats.len(), 2010);
    assert_eq!(merged.playback_stat_buckets.len(), 2);
    let again = three_way_merge(&merged, &merged, 0, &HashMap::new());
    assert_eq!(again.playback_stats.len(), 2010);
    assert_eq!(again.playback_stat_buckets.len(), 2);
}

/// Y2 回归：legacy 迁移合成的 addedAt（无 membership token）不得据此裁墓碑，
/// 否则已删歌会在所有端复活（对齐 Android SyncPlaylistDeletionPolicy P1-1）
#[test]
fn legacy_tombstone_survives_active_song_without_membership_tokens() {
    let active = song("42", 300);
    assert!(active.sync_membership_tokens.is_empty());
    let deletion = SyncPlaylistSongDeletion {
        playlist_id: "1".into(),
        song_id: "42".into(),
        album: "netease".into(),
        deleted_at: 200,
        device_id: "phone".into(),
        ..Default::default()
    };

    let pruned = prune_playlist_song_deletions(&[deletion], &[playlist(vec![active])]);

    assert_eq!(pruned.len(), 1, "无 token 的活跃歌不构成\"重新添加\"证据");
}

#[test]
fn legacy_tombstone_is_pruned_when_readded_song_carries_membership_token() {
    let mut readded = song("42", 300);
    readded.sync_membership_tokens = vec![SyncCausalToken {
        device_id: "desktop".into(),
        counter: 7,
    }];
    let deletion = SyncPlaylistSongDeletion {
        playlist_id: "1".into(),
        song_id: "42".into(),
        album: "netease".into(),
        deleted_at: 200,
        device_id: "phone".into(),
        ..Default::default()
    };

    let pruned = prune_playlist_song_deletions(&[deletion], &[playlist(vec![readded])]);

    assert!(
        pruned.is_empty(),
        "带 token 且 added_at 更晚才算真正重新添加"
    );
}

#[test]
fn counter_shard_totals_saturate_before_overflow() {
    for (listen, count) in [(i64::MAX, 1), (1, i32::MAX)] {
        let local = SyncTrackStat {
            identity_key: "k".into(),
            total_listen_ms: listen,
            play_count: count,
            first_played_at: 100,
            last_played_at: 100,
            counter_shards: vec![SyncPlaybackCounterShard {
                device_id: "desktop".into(),
                total_listen_ms: listen,
                play_count: count,
                first_played_at: 100,
                last_played_at: 100,
                ..Default::default()
            }],
            ..Default::default()
        };
        let mut remote = local.clone();
        remote.counter_shards[0].device_id = "android".into();
        let stats = merge_playback_stats(
            std::slice::from_ref(&local),
            std::slice::from_ref(&remote),
            0,
        );
        let buckets = merge_stat_buckets(
            &[bucket_from_clear_stat(&local)],
            &[bucket_from_clear_stat(&remote)],
            0,
        );
        let expected_listen = listen.saturating_add(listen);
        let expected_count = count.saturating_add(count);
        assert_eq!(stats[0].total_listen_ms, expected_listen);
        assert_eq!(stats[0].play_count, expected_count);
        assert_eq!(buckets[0].total_listen_ms, expected_listen);
        assert_eq!(buckets[0].play_count, expected_count);
        assert_eq!(stats[0].counter_shards.len(), 2);
        let again = merge_playback_stats(&stats, &stats, 0);
        assert_eq!(again[0].total_listen_ms, expected_listen);
        assert_eq!(again[0].play_count, expected_count);
    }
}

/// Y3 回归：合并后分片为空时 counter_base 必须归零（对齐 Android），
/// 否则 base=total 与对端的 0 每轮互相"纠正"，回声上传永不收敛
#[test]
fn counter_base_is_zeroed_when_merged_shards_are_empty() {
    let local = SyncTrackStat {
        identity_key: "k".into(),
        total_listen_ms: 900,
        play_count: 9,
        last_played_at: 100,
        ..Default::default()
    };
    let mut remote = local.clone();
    remote.total_listen_ms = 700;
    remote.play_count = 7;
    remote.last_played_at = 90;

    let merged = merge_playback_stats(&[local], &[remote], 0);

    assert_eq!(merged.len(), 1);
    assert_eq!(merged[0].total_listen_ms, 900);
    assert_eq!(merged[0].play_count, 9);
    assert_eq!(merged[0].counter_base_listen_ms, 0);
    assert_eq!(merged[0].counter_base_play_count, 0);
}

/// Y6 回归：双方都改名固定 local-wins（对齐 Android），不再按 modified_at 裁决
#[test]
fn playlist_rename_conflict_prefers_local_when_both_sides_changed() {
    let mut local = playlist(Vec::new());
    local.name = "Local name".into();
    local.modified_at = 300;
    let mut remote = playlist(Vec::new());
    remote.name = "Remote name".into();
    // 远端时间戳更晚（快钟设备）也不能赢
    remote.modified_at = 400;

    let merged = three_way_merge(
        &sync_data(vec![local]),
        &sync_data(vec![remote]),
        200,
        &HashMap::new(),
    );

    assert_eq!(merged.playlists[0].name, "Local name");
}

/// Y7 回归：已删歌单不参与来源顺序，墓碑统一补在末尾（对齐 Android filterNot）
#[test]
fn deleted_playlists_are_appended_after_active_ones_in_merge_order() {
    let mut tombstone = playlist(Vec::new());
    tombstone.id = "2".into();
    tombstone.is_deleted = true;
    tombstone.modified_at = 300;
    let active = playlist(Vec::new());
    let local = sync_data(vec![tombstone.clone(), active.clone()]);
    let remote = sync_data(vec![tombstone, active]);

    let merged = three_way_merge(&local, &remote, 200, &HashMap::new());

    assert_eq!(
        merged
            .playlists
            .iter()
            .map(|playlist| (playlist.id.as_str(), playlist.is_deleted))
            .collect::<Vec<_>>(),
        vec![("1", false), ("2", true)],
    );
}

fn stat_bucket(day_start_at: i64, identity_key: &str) -> SyncPlaybackStatBucket {
    SyncPlaybackStatBucket {
        day_start_at,
        identity_key: identity_key.into(),
        play_count: 1,
        last_played_at: day_start_at,
        ..Default::default()
    }
}

fn sync_data(playlists: Vec<SyncPlaylist>) -> SyncData {
    SyncData {
        playlists,
        ..Default::default()
    }
}

fn playlist(songs: Vec<SyncSong>) -> SyncPlaylist {
    SyncPlaylist {
        id: "1".into(),
        name: "Playlist".into(),
        songs,
        created_at: 1,
        modified_at: 20,
        is_deleted: false,
        song_order_version: DISPLAY_ORDER_SONG_ORDER_VERSION,
    }
}

fn song(id: &str, added_at: i64) -> SyncSong {
    SyncSong {
        id: id.into(),
        name: "Song".into(),
        album: "netease".into(),
        channel_id: Some("netease".into()),
        audio_id: Some(id.into()),
        added_at,
        sync_metadata_version: CURRENT_SYNC_METADATA_VERSION,
        ..Default::default()
    }
}

fn hour_total(device: &str, epoch: i64, local_hour: i32, total: i64) -> SyncPlaybackHourTotal {
    SyncPlaybackHourTotal {
        device_id: device.into(),
        epoch_started_at: epoch,
        hour_start_at: 0,
        local_hour,
        total_listen_ms: total,
    }
}

// 对齐 Android SyncPlaybackStatsMergePolicyTest「hourly amounts converge …」
#[test]
fn hourly_amounts_converge_across_devices_retries_old_peers_and_clear_epochs() {
    let first = hour_total("phone", 0, 8, 14);
    let later = hour_total("phone", 0, 8, 26);
    let tablet = hour_total("tablet", 0, 8, 26);
    let next_epoch = hour_total("phone", 100, 8, 9);
    let shard = |epoch: i64, listen: i64, first: i64, last: i64| SyncPlaybackCounterShard {
        device_id: "phone".into(),
        epoch_started_at: epoch,
        total_listen_ms: listen,
        play_count: 1,
        first_played_at: first,
        last_played_at: last,
        ..Default::default()
    };
    let local = SyncPlaybackStatBucket {
        identity_key: "song".into(),
        first_played_at: 20,
        last_played_at: 114,
        counter_shards: vec![shard(0, 26, 20, 46), shard(100, 9, 105, 114)],
        hourly_totals: vec![next_epoch.clone(), first],
        ..Default::default()
    };
    let remote = SyncPlaybackStatBucket {
        hourly_totals: vec![tablet.clone(), later.clone()],
        ..local.clone()
    };
    let merge = |a: &SyncPlaybackStatBucket, b: &SyncPlaybackStatBucket, cleared_at: i64| {
        let mut merged =
            merge_stat_buckets(std::slice::from_ref(a), std::slice::from_ref(b), cleared_at);
        assert_eq!(merged.len(), 1);
        merged.remove(0)
    };

    let merged = merge(&local, &remote, 0);
    assert_eq!(
        merged.hourly_totals,
        vec![later, next_epoch.clone(), tablet]
    );
    assert_eq!(
        merge(&remote, &local, 0).hourly_totals,
        merged.hourly_totals
    );
    assert_eq!(
        merge(&merged, &remote, 0).hourly_totals,
        merged.hourly_totals
    );
    let without_hours = SyncPlaybackStatBucket {
        hourly_totals: Vec::new(),
        ..merged.clone()
    };
    assert_eq!(
        merge(&merged, &without_hours, 0).hourly_totals,
        merged.hourly_totals
    );

    let after_clear = merge(&merged, &remote, 100);
    assert_eq!(after_clear.hourly_totals, vec![next_epoch]);
    assert_eq!(after_clear.total_listen_ms, 9);
}

#[test]
fn recorded_bucket_and_shard_dates_merge_to_the_earlier_valid_day() {
    let bucket = |date: Option<&str>, shard_date: Option<&str>| SyncPlaybackStatBucket {
        identity_key: "song".into(),
        play_count: 1,
        last_played_at: 10,
        recorded_local_date: date.map(Into::into),
        counter_shards: vec![SyncPlaybackCounterShard {
            device_id: "phone".into(),
            play_count: 1,
            first_played_at: 10,
            last_played_at: 10,
            recorded_local_date: shard_date.map(Into::into),
            ..Default::default()
        }],
        ..Default::default()
    };
    let merged = merge_stat_buckets(
        &[bucket(Some("2026-11-25"), Some("2026-11-25"))],
        &[bucket(Some("2026-11-24"), None)],
        0,
    );
    assert_eq!(merged[0].recorded_local_date.as_deref(), Some("2026-11-24"));
    assert_eq!(
        merged[0].counter_shards[0].recorded_local_date.as_deref(),
        Some("2026-11-25")
    );
    let invalid = merge_stat_buckets(&[bucket(Some("2026-02-30"), Some("bad"))], &[], 0);
    assert!(invalid[0].recorded_local_date.is_none());
    assert!(invalid[0].counter_shards[0].recorded_local_date.is_none());
}

#[test]
fn hourly_only_changes_trigger_an_upload_and_dirty_rows_do_not() {
    let base = SyncPlaybackStatBucket {
        identity_key: "song".into(),
        play_count: 1,
        last_played_at: 10,
        hourly_totals: vec![hour_total("phone", 0, 8, 14), hour_total("phone", 0, 9, 3)],
        ..Default::default()
    };
    let data = |bucket: SyncPlaybackStatBucket| SyncData {
        playback_stat_buckets: vec![bucket],
        ..Default::default()
    };
    let remote = data(base.clone());
    let mut dirty = base.clone();
    dirty.hourly_totals.reverse();
    dirty.hourly_totals.push(hour_total(" ", 0, 8, 99));
    assert!(!has_data_changed(&remote, &data(dirty)));
    let mut grown = base.clone();
    grown.hourly_totals[0].total_listen_ms = 15;
    assert!(has_data_changed(&remote, &data(grown)));
    assert!(has_data_changed(
        &remote,
        &data(SyncPlaybackStatBucket {
            hourly_totals: Vec::new(),
            ..base
        })
    ));
}
