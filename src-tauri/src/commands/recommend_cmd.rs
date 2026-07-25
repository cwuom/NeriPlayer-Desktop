// 推荐 & 用户数据命令
use crate::error::{AppError, AppResult};
use crate::state::AppState;
use serde_json::Value;
use tauri::State;

/// 获取网易云个性化推荐歌单（需登录）
#[tauri::command]
pub async fn get_recommended_playlists(
    limit: Option<u32>,
    state: State<'_, AppState>,
) -> AppResult<Value> {
    let client = crate::api::netease::client::NeteaseClient::new(&state.http());
    client.get_recommended_playlists(limit.unwrap_or(30)).await
}

/// 获取网易云每日推荐歌曲（需登录）
#[tauri::command]
pub async fn get_recommended_songs(state: State<'_, AppState>) -> AppResult<Value> {
    let client = crate::api::netease::client::NeteaseClient::new(&state.http());
    client.get_recommended_songs().await
}

/// 获取用户歌单列表（多平台）
#[tauri::command]
pub async fn get_user_playlists(platform: String, state: State<'_, AppState>) -> AppResult<Value> {
    match platform.as_str() {
        "netease" => {
            // 在独立作用域内取值，确保 MutexGuard 在 await 前释放
            let uid = {
                let auth = state.auth.lock();
                auth.netease
                    .as_ref()
                    .and_then(|a| a.user_id)
                    .ok_or_else(|| AppError::Api("Netease not logged in".into()))?
            };
            let client = crate::api::netease::client::NeteaseClient::new(&state.http());
            client.get_user_playlists(uid, 1000, 0).await
        }
        "bilibili" => {
            let mid = {
                let auth = state.auth.lock();
                auth.bilibili
                    .as_ref()
                    .and_then(|a| a.mid)
                    .ok_or_else(|| AppError::Api("Bilibili not logged in".into()))?
            };
            let client = crate::api::bilibili::client::BiliClient::new(
                &state.http(),
                state.cookie_jar.clone(),
            );
            let mut resp = client.get_user_favorites(mid).await?;

            // For folders with empty cover but non-zero media_count,
            // concurrently fetch the first item's cover as fallback.
            if let Some(list) = resp["data"]["list"].as_array() {
                let mut futures = Vec::new();
                for (idx, folder) in list.iter().enumerate() {
                    let cover = folder["cover"].as_str().unwrap_or("");
                    let media_count = folder["media_count"].as_u64().unwrap_or(0);
                    if cover.is_empty() && media_count > 0 {
                        if let Some(media_id) = folder["id"].as_u64() {
                            let c = crate::api::bilibili::client::BiliClient::new(
                                &state.http(),
                                state.cookie_jar.clone(),
                            );
                            futures.push(async move {
                                let pic = c
                                    .get_fav_folder_info(media_id)
                                    .await
                                    .ok()
                                    .and_then(|r| {
                                        r["data"]["cover"].as_str().map(|s| s.to_string())
                                    })
                                    .unwrap_or_default();
                                (idx, pic)
                            });
                        }
                    }
                }

                let results = futures_util::future::join_all(futures).await;
                if let Some(list) = resp["data"]["list"].as_array_mut() {
                    for (idx, cover) in results {
                        if !cover.is_empty() {
                            if let Some(folder) = list.get_mut(idx) {
                                folder["cover"] = serde_json::Value::String(cover);
                            }
                        }
                    }
                }
            }

            Ok(resp)
        }
        "youtube" => {
            let yt_auth = {
                let auth = state.auth.lock();
                auth.youtube
                    .as_ref()
                    .ok_or_else(|| AppError::Api("YouTube not logged in".into()))?
                    .clone()
            };
            let client = crate::api::youtube::client::YouTubeClient::new(&state.http());
            client.get_library_playlists(&yt_auth).await
        }
        _ => Err(AppError::Api(format!("Unknown platform: {}", platform))),
    }
}

/// 获取用户账号信息（多平台）
#[tauri::command]
pub async fn get_user_account(platform: String, state: State<'_, AppState>) -> AppResult<Value> {
    match platform.as_str() {
        "netease" => {
            let client = crate::api::netease::client::NeteaseClient::new(&state.http());
            client.get_user_account().await
        }
        "bilibili" => {
            let client = crate::api::bilibili::client::BiliClient::new(
                &state.http(),
                state.cookie_jar.clone(),
            );
            client.get_user_info().await
        }
        _ => Err(AppError::Api(format!("Unsupported platform: {}", platform))),
    }
}

/// 获取 YouTube Music 首页信息流（需登录）
#[tauri::command]
pub async fn get_home_feed(app: tauri::AppHandle, state: State<'_, AppState>) -> AppResult<Value> {
    // 加载前机会式保鲜 YouTube 会话(冷却/熔断内会自动跳过)
    crate::commands::auth_cmd::maybe_refresh_youtube_session(&app, state.inner(), false).await;
    let yt_auth = {
        let auth = state.auth.lock();
        auth.youtube
            .as_ref()
            .ok_or_else(|| AppError::Api("YouTube not logged in".into()))?
            .clone()
    };
    let client = crate::api::youtube::client::YouTubeClient::new(&state.http());
    client.get_home_feed(&yt_auth).await
}

/// 获取网易云精品歌单（按分类，无需登录）
#[tauri::command]
pub async fn get_high_quality_playlists(
    cat: Option<String>,
    limit: Option<u32>,
    state: State<'_, AppState>,
) -> AppResult<Value> {
    let client = crate::api::netease::client::NeteaseClient::new(&state.http());
    client
        .get_high_quality_playlists(cat.as_deref().unwrap_or("全部"), limit.unwrap_or(30))
        .await
}

/// 获取精品歌单分类标签
#[tauri::command]
pub async fn get_high_quality_tags(state: State<'_, AppState>) -> AppResult<Value> {
    let client = crate::api::netease::client::NeteaseClient::new(&state.http());
    client.get_high_quality_tags().await
}

/// 喜欢/取消喜欢歌曲（网易云）
#[tauri::command]
pub async fn like_song(song_id: u64, like: bool, state: State<'_, AppState>) -> AppResult<Value> {
    let client = crate::api::netease::client::NeteaseClient::new(&state.http());
    client.like_song(song_id, like).await
}

/// 获取用户喜欢的歌曲 ID 列表（网易云）
#[tauri::command]
pub async fn get_liked_song_ids(state: State<'_, AppState>) -> AppResult<Value> {
    let uid = {
        let auth = state.auth.lock();
        auth.netease
            .as_ref()
            .and_then(|a| a.user_id)
            .ok_or_else(|| AppError::Api("Netease not logged in".into()))?
    };
    let client = crate::api::netease::client::NeteaseClient::new(&state.http());
    client.get_liked_song_ids(uid).await
}

/// 获取专辑详情（网易云）
#[tauri::command]
pub async fn get_album_detail(album_id: u64, state: State<'_, AppState>) -> AppResult<Value> {
    let client = crate::api::netease::client::NeteaseClient::new(&state.http());
    client.get_album_detail(album_id).await
}

/// 获取歌曲详情（网易云）
#[tauri::command]
pub async fn get_netease_song_detail(song_id: u64, state: State<'_, AppState>) -> AppResult<Value> {
    let client = crate::api::netease::client::NeteaseClient::new(&state.http());
    client.get_song_detail(&[song_id]).await
}

/// 获取用户收藏的专辑列表（网易云）
#[tauri::command]
pub async fn get_user_stared_albums(
    offset: Option<u32>,
    limit: Option<u32>,
    state: State<'_, AppState>,
) -> AppResult<Value> {
    let client = crate::api::netease::client::NeteaseClient::new(&state.http());
    client
        .get_user_stared_albums(offset.unwrap_or(0), limit.unwrap_or(1000))
        .await
}

/// 获取 B站收藏夹详情
#[tauri::command]
pub async fn get_bili_fav_folder_info(
    media_id: u64,
    state: State<'_, AppState>,
) -> AppResult<Value> {
    let client = crate::api::bilibili::client::BiliClient::new(
        &state.http(),
        state.cookie_jar.clone(),
    );
    client.get_fav_folder_info(media_id).await
}

/// 获取 B站收藏夹内容
#[tauri::command]
pub async fn get_bili_favorite_items(
    media_id: u64,
    page: Option<u32>,
    state: State<'_, AppState>,
) -> AppResult<Value> {
    let client = crate::api::bilibili::client::BiliClient::new(
        &state.http(),
        state.cookie_jar.clone(),
    );
    client.get_favorite_items(media_id, page.unwrap_or(1)).await
}

/// 获取网易云歌单完整详情（含大歌单分页补全）
#[tauri::command]
pub async fn get_netease_playlist_detail(
    playlist_id: u64,
    state: State<'_, AppState>,
) -> AppResult<Value> {
    let client = crate::api::netease::client::NeteaseClient::new(&state.http());
    let mut detail = client.get_playlist(playlist_id).await?;

    // 若 trackIds 数量 > tracks 数量，分页补全
    let track_ids: Vec<u64> = detail["playlist"]["trackIds"]
        .as_array()
        .map(|arr| arr.iter().filter_map(|v| v["id"].as_u64()).collect())
        .unwrap_or_default();

    let existing_count = detail["playlist"]["tracks"]
        .as_array()
        .map(|a| a.len())
        .unwrap_or(0);

    if track_ids.len() > existing_count {
        // 取缺失部分的 ID，每批 300 个
        let missing_ids: Vec<u64> = track_ids[existing_count..].to_vec();
        let mut all_extra_songs: Vec<Value> = Vec::new();

        for chunk in missing_ids.chunks(300) {
            let batch = client.get_song_detail(chunk).await?;
            if let Some(songs) = batch["songs"].as_array() {
                all_extra_songs.extend(songs.iter().cloned());
            }
        }

        // 合并到 tracks 数组
        if let Some(tracks) = detail["playlist"]["tracks"].as_array_mut() {
            tracks.extend(all_extra_songs);
        }
    }

    Ok(detail)
}

/// 单次歌单详情最多展开的 continuation 页数
const YOUTUBE_PLAYLIST_MAX_PAGES: usize = 20;

/// 获取 YouTube Music 歌单详情(自动展开 continuation 分页 + 会话刷新)
#[tauri::command]
pub async fn get_youtube_playlist_detail(
    app: tauri::AppHandle,
    browse_id: String,
    state: State<'_, AppState>,
) -> AppResult<Value> {
    // 加载前机会式保鲜 YouTube 会话(冷却/熔断内会自动跳过)
    crate::commands::auth_cmd::maybe_refresh_youtube_session(&app, state.inner(), false).await;
    let yt_auth = {
        let auth = state.auth.lock();
        auth.youtube
            .as_ref()
            .ok_or_else(|| AppError::Api("YouTube not logged in".into()))?
            .clone()
    };
    let browse_id = crate::api::youtube::playlist::ensure_browse_id(&browse_id)?;
    let client = crate::api::youtube::client::YouTubeClient::new(&state.http());
    let (detail, refreshed_auth) = crate::api::youtube::playlist::fetch_playlist_detail_pages(
        &client,
        &browse_id,
        &yt_auth,
        YOUTUBE_PLAYLIST_MAX_PAGES,
    )
    .await?;

    // 会话保鲜: 落盘 + 注入共享 jar, 与 maybe_refresh_youtube_session 一致
    if let Some(updated) = refreshed_auth {
        let _cookie_guard = state.auth_cookie_gate.lock().await;
        let mut auth_state = state.auth.lock();
        if let Some(saved) = auth_state.youtube.as_ref() {
            if crate::commands::auth_cmd::youtube_auth_matches(saved, &updated) {
                let refreshed_cookies = updated.cookies.clone();
                let mut updated_state = auth_state.clone();
                updated_state.youtube = Some(updated);
                match crate::auth::cookies::save_auth_strict(&app, &updated_state) {
                    Ok(()) => {
                        *auth_state = updated_state;
                        crate::auth::cookies::inject_cookies(
                            &state.cookie_jar,
                            &refreshed_cookies,
                        );
                    }
                    Err(error) => {
                        log::error!(target: "youtube-refresh", "failed to persist playlist session refresh: {error}");
                    }
                }
            }
        }
    }

    Ok(detail)
}

/// 验证平台登录状态是否仍有效
#[tauri::command]
pub async fn validate_auth(platform: String, state: State<'_, AppState>) -> AppResult<bool> {
    match platform.as_str() {
        "netease" => {
            let client = crate::api::netease::client::NeteaseClient::new(&state.http());
            match client.get_user_account().await {
                Ok(v) => Ok(v["profile"]["userId"].as_u64().is_some()),
                Err(_) => Ok(false),
            }
        }
        "bilibili" => {
            let client = crate::api::bilibili::client::BiliClient::new(
                &state.http(),
                state.cookie_jar.clone(),
            );
            client.validate_session().await
        }
        "youtube" => {
            // YouTube 通过尝试加载首页来验证
            let yt_auth = {
                let auth = state.auth.lock();
                auth.youtube.as_ref().cloned()
            };
            match yt_auth {
                Some(auth) => {
                    let client = crate::api::youtube::client::YouTubeClient::new(&state.http());
                    match client.get_home_feed(&auth).await {
                        Ok(_) => Ok(true),
                        Err(_) => Ok(false),
                    }
                }
                None => Ok(false),
            }
        }
        _ => Err(AppError::Api(format!("Unknown platform: {}", platform))),
    }
}
