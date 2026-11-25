//! 收听时长按本地小时切分，对齐 Android PlaybackListeningHours
//!
//! 槽位键为（设备, epoch, 小时起点, 本地小时），只由本机单调累加，
//! 跨端合并直接取最大值（见 merge_sync_playback_hours）

use chrono::{Offset, TimeZone, Timelike};

use crate::sync::models::{merge_sync_playback_hours, SyncPlaybackHourTotal};

const MILLIS_PER_HOUR: i64 = 60 * 60 * 1000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct ListeningHour {
    pub hour_start_at: i64,
    pub local_hour: i32,
    pub duration_ms: i64,
}

fn offset_ms<T: TimeZone>(zone: &T, at: i64) -> Option<i64> {
    let moment = zone.timestamp_millis_opt(at).single()?;
    Some(i64::from(moment.offset().fix().local_minus_utc()) * 1000)
}

/// (low, high] 内第一个满足条件的时刻；调用方保证 low 不满足、high 满足
fn first_where(mut low: i64, mut high: i64, matches: impl Fn(i64) -> bool) -> i64 {
    while high - low > 1 {
        let middle = low + (high - low) / 2;
        if matches(middle) {
            high = middle;
        } else {
            low = middle;
        }
    }
    high
}

/// 把截至 end_at 的一段收听切到各个本地小时
pub(super) fn listening_hours<T: TimeZone>(
    zone: &T,
    end_at: i64,
    listened_ms: i64,
) -> Vec<ListeningHour> {
    let mut hours = Vec::new();
    if listened_ms < 1 || listened_ms > end_at {
        return hours;
    }
    let mut cursor = end_at - listened_ms;
    while cursor < end_at {
        let Some(moment) = zone.timestamp_millis_opt(cursor).single() else {
            break;
        };
        let offset = i64::from(moment.offset().fix().local_minus_utc()) * 1000;
        let same_offset = |at: i64| offset_ms(zone, at) == Some(offset);
        let top_of_hour = (cursor + offset).div_euclid(MILLIS_PER_HOUR) * MILLIS_PER_HOUR;
        let nominal_start = top_of_hour - offset;
        // 某些地区在半点切换偏移：这一小时实际从切换时刻起算
        let hour_start = if same_offset(nominal_start) {
            nominal_start
        } else {
            first_where(nominal_start, cursor, same_offset)
        };
        let next_hour = top_of_hour + MILLIS_PER_HOUR - offset;
        let transition = if same_offset(next_hour - 1) {
            i64::MAX
        } else {
            first_where(cursor, next_hour - 1, |at| !same_offset(at))
        };
        let next = end_at.min(next_hour).min(transition);
        hours.push(ListeningHour {
            hour_start_at: hour_start.max(0),
            local_hour: moment.hour() as i32,
            duration_ms: next - cursor,
        });
        cursor = next;
    }
    hours
}

/// 一次收听增量对应的小时槽位，对齐 Android PlaybackStatsPendingDeltaEntity.hourIncrements
///
/// 跨零点的收听整体记在当天，起点被截到当天零点
pub(super) fn hour_increments<T: TimeZone>(
    zone: &T,
    device_id: &str,
    epoch_started_at: i64,
    day_start_at: i64,
    played_at: i64,
    listened_ms: i64,
) -> Vec<SyncPlaybackHourTotal> {
    if listened_ms <= 0 {
        return Vec::new();
    }
    let start = day_start_at.max(played_at.saturating_sub(listened_ms));
    listening_hours(zone, start.saturating_add(listened_ms), listened_ms)
        .into_iter()
        .map(|hour| SyncPlaybackHourTotal {
            device_id: device_id.to_string(),
            epoch_started_at,
            hour_start_at: day_start_at.max(hour.hour_start_at),
            local_hour: hour.local_hour,
            total_listen_ms: hour.duration_ms,
        })
        .collect()
}

/// 把增量累加进已有小时数据，对齐 Android addHourIncrements
pub(super) fn add_hour_increments(
    totals: &mut Vec<SyncPlaybackHourTotal>,
    increments: Vec<SyncPlaybackHourTotal>,
) {
    if increments.is_empty() {
        return;
    }
    for increment in increments {
        match totals
            .iter_mut()
            .find(|total| total.slot() == increment.slot())
        {
            Some(total) => {
                total.total_listen_ms = total
                    .total_listen_ms
                    .saturating_add(increment.total_listen_ms)
            }
            None => totals.push(increment),
        }
    }
    *totals = merge_sync_playback_hours(totals, &[], 0);
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::FixedOffset;

    const HOUR: i64 = MILLIS_PER_HOUR;

    #[test]
    fn listening_is_split_at_local_hour_boundaries() {
        let zone = FixedOffset::east_opt(8 * 3600).unwrap();
        // UTC 01:40 → 本地 09:40，收听 50 分钟跨到 10:30
        let end = 2 * HOUR + 30 * 60_000;
        let hours = listening_hours(&zone, end, 50 * 60_000);
        assert_eq!(
            hours,
            vec![
                ListeningHour {
                    hour_start_at: HOUR,
                    local_hour: 9,
                    duration_ms: 20 * 60_000
                },
                ListeningHour {
                    hour_start_at: 2 * HOUR,
                    local_hour: 10,
                    duration_ms: 30 * 60_000
                },
            ]
        );
    }

    #[test]
    fn half_hour_offsets_use_their_own_hour_start() {
        let zone = FixedOffset::east_opt(5 * 3600 + 1800).unwrap();
        // 本地 12:10 结束，收听 20 分钟：11:50 起
        let local_noon = 12 * HOUR - (5 * HOUR + HOUR / 2);
        let end = local_noon + 10 * 60_000;
        let hours = listening_hours(&zone, end, 20 * 60_000);
        assert_eq!(hours.len(), 2);
        assert_eq!(
            (hours[0].local_hour, hours[0].hour_start_at),
            (11, local_noon - HOUR)
        );
        assert_eq!(
            (hours[1].local_hour, hours[1].hour_start_at),
            (12, local_noon)
        );
        assert_eq!(
            hours.iter().map(|hour| hour.duration_ms).sum::<i64>(),
            20 * 60_000
        );
    }

    #[test]
    fn invalid_durations_produce_no_hours() {
        let zone = FixedOffset::east_opt(0).unwrap();
        assert!(listening_hours(&zone, 10, 0).is_empty());
        assert!(listening_hours(&zone, 10, 11).is_empty());
        assert!(hour_increments(&zone, "pc", 0, 0, 10, -1).is_empty());
    }

    #[test]
    fn increments_crossing_midnight_stay_in_the_recorded_day() {
        let zone = FixedOffset::east_opt(0).unwrap();
        let day = 24 * HOUR;
        // 23:50 开始听到次日 00:10，次日分桶只记零点之后的部分起点
        let rows = hour_increments(&zone, "pc", 7, day, day + 10 * 60_000, 20 * 60_000);
        assert_eq!(
            rows,
            vec![SyncPlaybackHourTotal {
                device_id: "pc".into(),
                epoch_started_at: 7,
                hour_start_at: day,
                local_hour: 0,
                total_listen_ms: 20 * 60_000,
            }]
        );
    }

    #[test]
    fn repeated_increments_accumulate_in_the_same_slot() {
        let zone = FixedOffset::east_opt(0).unwrap();
        let mut totals = vec![SyncPlaybackHourTotal {
            device_id: "phone".into(),
            epoch_started_at: 0,
            hour_start_at: 3 * HOUR,
            local_hour: 3,
            total_listen_ms: 99,
        }];
        for minute in [10, 20] {
            let played_at = 3 * HOUR + minute * 60_000;
            add_hour_increments(
                &mut totals,
                hour_increments(&zone, "pc", 0, 0, played_at, 60_000),
            );
        }
        assert_eq!(totals.len(), 2);
        let own = totals.iter().find(|row| row.device_id == "pc").unwrap();
        assert_eq!((own.local_hour, own.total_listen_ms), (3, 120_000));
        assert_eq!(
            totals
                .iter()
                .find(|row| row.device_id == "phone")
                .unwrap()
                .total_listen_ms,
            99
        );
    }
}
