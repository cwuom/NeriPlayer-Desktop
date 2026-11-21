use std::collections::BTreeMap;

use rusqlite::{Connection, Transaction};
use serde::Serialize;
use serde_json::{Map, Value};

use neri_common::error::{AppError, AppResult};
use neri_platform::api::bilibili::client::BiliVideoInfo;

const RULES_KEY: &str = "biliVideoSkipRules";
const MAX_INTERVALS: usize = 100;
const MAX_RULES: usize = 2000;

pub use neri_model::bili_video_skip::{BiliVideoSkipInterval, BiliVideoSkipRule};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BiliVideoSkipTargetOption {
    pub bvid: String,
    pub cid: i64,
    pub label: String,
    pub duration_ms: i64,
}

pub fn target_options(video: &BiliVideoInfo) -> Vec<BiliVideoSkipTargetOption> {
    let bvid = video.bvid.trim();
    if bvid.is_empty() {
        return Vec::new();
    }
    video
        .pages
        .iter()
        .filter_map(|page| {
            let cid = i64::try_from(page.cid).ok().filter(|cid| *cid > 0)?;
            Some(BiliVideoSkipTargetOption {
                bvid: bvid.to_string(),
                cid,
                label: if page.part.trim().is_empty() {
                    format!("P{}", page.page)
                } else {
                    page.part.trim().to_string()
                },
                duration_ms: i64::try_from(page.duration.saturating_mul(1000)).unwrap_or(i64::MAX),
            })
        })
        .collect()
}

pub fn normalize_intervals(
    intervals: &[BiliVideoSkipInterval],
    duration_ms: i64,
) -> Vec<BiliVideoSkipInterval> {
    let maximum = if duration_ms > 0 {
        duration_ms
    } else {
        i64::MAX
    };
    let mut sorted: Vec<_> = intervals
        .iter()
        .map(|interval| BiliVideoSkipInterval {
            start_ms: interval.start_ms.max(0),
            end_ms: interval.end_ms.min(maximum),
        })
        .filter(|interval| interval.end_ms > interval.start_ms)
        .collect();
    sorted.sort_by_key(|interval| (interval.start_ms, interval.end_ms));
    sorted.truncate(MAX_INTERVALS);
    let mut result: Vec<BiliVideoSkipInterval> = Vec::new();
    for interval in sorted {
        if let Some(previous) = result
            .last_mut()
            .filter(|previous| interval.start_ms <= previous.end_ms)
        {
            previous.end_ms = previous.end_ms.max(interval.end_ms);
        } else {
            result.push(interval);
        }
    }
    result
}

pub fn normalize_rules(rules: &[BiliVideoSkipRule]) -> Vec<BiliVideoSkipRule> {
    let mut by_target: BTreeMap<(String, i64), BiliVideoSkipRule> = BTreeMap::new();
    for rule in rules {
        let bvid = rule.bvid.trim();
        if bvid.is_empty() || rule.cid <= 0 {
            continue;
        }
        let mut rule = rule.clone();
        rule.bvid = bvid.to_string();
        rule.modified_at = rule.modified_at.max(0);
        rule.intervals = if rule.is_deleted {
            Vec::new()
        } else {
            normalize_intervals(&rule.intervals, 0)
        };
        if !rule.is_deleted && rule.intervals.is_empty() {
            continue;
        }
        let key = (rule.bvid.clone(), rule.cid);
        match by_target.get_mut(&key) {
            Some(previous) if previous.modified_at > rule.modified_at => {}
            Some(previous) if previous.modified_at == rule.modified_at => {
                if previous.is_deleted && !rule.is_deleted {
                    *previous = rule;
                } else if !previous.is_deleted && !rule.is_deleted {
                    previous.intervals.extend(rule.intervals);
                    previous.intervals = normalize_intervals(&previous.intervals, 0);
                }
            }
            _ => {
                by_target.insert(key, rule);
            }
        }
    }
    by_target.into_values().take(MAX_RULES).collect()
}

pub fn rules_from_extensions(extensions: &Map<String, Value>) -> AppResult<Vec<BiliVideoSkipRule>> {
    match extensions.get(RULES_KEY) {
        Some(value) => Ok(normalize_rules(&serde_json::from_value::<
            Vec<BiliVideoSkipRule>,
        >(value.clone())?)),
        None => Ok(Vec::new()),
    }
}

pub fn load_from(connection: &Connection) -> AppResult<Vec<BiliVideoSkipRule>> {
    rules_from_extensions(&crate::sync::storage::load_archive_metadata(connection)?.extensions)
}

pub fn set(
    transaction: &Transaction<'_>,
    bvid: &str,
    cid: i64,
    intervals: &[BiliVideoSkipInterval],
    duration_ms: i64,
    now: i64,
) -> AppResult<(Option<BiliVideoSkipRule>, bool)> {
    let bvid = bvid.trim();
    if bvid.is_empty() || cid <= 0 {
        return Err(AppError::Other(
            "Bili video skip target must contain a BVID and CID".into(),
        ));
    }
    let intervals = normalize_intervals(intervals, duration_ms);
    let mut output = None;
    let mut changed = false;
    crate::sync::storage::update_archive_extensions(transaction, |extensions| {
        let mut rules = rules_from_extensions(extensions)?;
        let previous = rules
            .iter()
            .find(|rule| rule.bvid == bvid && rule.cid == cid);
        let is_deleted = intervals.is_empty();
        if previous.is_none() && is_deleted {
            return Ok(());
        }
        if let Some(previous) =
            previous.filter(|rule| rule.is_deleted == is_deleted && rule.intervals == intervals)
        {
            output = Some(previous.clone());
            return Ok(());
        }
        let next_revision = previous.map_or(Ok(1), |rule| {
            rule.modified_at
                .checked_add(1)
                .ok_or_else(|| AppError::Other("Bili video skip revision overflow".into()))
        })?;
        let rule = BiliVideoSkipRule {
            bvid: bvid.to_string(),
            cid,
            intervals,
            modified_at: now.max(next_revision),
            is_deleted,
        };
        rules.retain(|previous| previous.bvid != bvid || previous.cid != cid);
        rules.push(rule.clone());
        let rules = normalize_rules(&rules);
        output = rules
            .iter()
            .find(|saved| saved.bvid == bvid && saved.cid == cid)
            .cloned();
        if output.is_none() {
            return Err(AppError::Other("Bili video skip rule limit reached".into()));
        }
        extensions.insert(RULES_KEY.into(), serde_json::to_value(rules)?);
        changed = true;
        Ok(())
    })?;
    Ok((output, changed))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::UserDatabase;
    use serde_json::json;

    fn interval(start_ms: i64, end_ms: i64) -> BiliVideoSkipInterval {
        BiliVideoSkipInterval { start_ms, end_ms }
    }

    #[test]
    fn intervals_follow_android_clamping_sort_limit_and_adjacent_merge() {
        assert_eq!(
            normalize_intervals(
                &[
                    interval(100, 300),
                    interval(-10, 150),
                    interval(400, 1000),
                    interval(300, 400),
                    interval(50, 0)
                ],
                500
            ),
            vec![interval(0, 500)]
        );
        let mut intervals: Vec<_> = (0..102)
            .rev()
            .map(|index| interval(index * 10, index * 10 + 1))
            .collect();
        intervals.push(interval(0, 10000));
        assert_eq!(normalize_intervals(&intervals, 0), vec![interval(0, 10000)]);
        assert_eq!(normalize_intervals(&intervals[..102], 0).len(), 100);
    }

    #[test]
    fn saves_keep_foreign_rules_and_archive_metadata_and_clear_with_newer_tombstone() {
        let database = UserDatabase::open_in_memory().unwrap();
        database
            .write(|transaction| {
                crate::sync::storage::update_archive_extensions(transaction, |extensions| {
                    extensions.insert(
                        "playlistUsageStats".into(),
                        json!([{"playlistKey":"foreign","openCount":2}]),
                    );
                    Ok(())
                })?;
                set(transaction, "BVforeign", 1, &[interval(1, 2)], 0, 50)?;
                let (first, changed) = set(
                    transaction,
                    " BVlocal ",
                    9,
                    &[interval(-1, 20), interval(20, 100)],
                    50,
                    100,
                )?;
                assert!(changed);
                assert_eq!(first.unwrap().intervals, vec![interval(0, 50)]);
                assert!(!set(transaction, "BVlocal", 9, &[interval(0, 50)], 0, 101)?.1);
                let (cleared, changed) = set(transaction, "BVlocal", 9, &[], 0, 100)?;
                assert!(changed);
                let cleared = cleared.unwrap();
                assert!(cleared.is_deleted && cleared.intervals.is_empty());
                assert_eq!(cleared.modified_at, 101);
                assert!(!set(transaction, "BVunknown", 10, &[], 0, 100)?.1);
                Ok(())
            })
            .unwrap();
        let rules = database.read(load_from).unwrap();
        assert_eq!(rules.len(), 2);
        assert_eq!(rules[0].bvid, "BVforeign");
        assert!(rules[1].is_deleted);
        let metadata = database
            .read(crate::sync::storage::load_archive_metadata)
            .unwrap();
        assert_eq!(
            metadata.extensions["playlistUsageStats"][0]["playlistKey"],
            "foreign"
        );
    }

    #[test]
    fn rules_resolve_android_ties_and_limits_without_losing_deletions() {
        let active = BiliVideoSkipRule {
            bvid: " BVtest ".into(),
            cid: 1,
            intervals: vec![interval(10, 20)],
            modified_at: 5,
            is_deleted: false,
        };
        let tombstone = BiliVideoSkipRule {
            is_deleted: true,
            ..active.clone()
        };
        let other = BiliVideoSkipRule {
            intervals: vec![interval(20, 30)],
            ..active.clone()
        };
        assert_eq!(
            normalize_rules(&[tombstone.clone(), active.clone(), other])[0].intervals,
            vec![interval(10, 30)]
        );
        assert!(
            normalize_rules(&[
                active.clone(),
                BiliVideoSkipRule {
                    modified_at: 6,
                    ..tombstone
                }
            ])[0]
                .is_deleted
        );
        let many: Vec<_> = (0..2001)
            .rev()
            .map(|index| BiliVideoSkipRule {
                bvid: format!("BV{index:04}"),
                ..active.clone()
            })
            .collect();
        let rules = normalize_rules(&many);
        assert_eq!(rules.len(), 2000);
        assert_eq!(rules[0].bvid, "BV0000");
        assert_eq!(rules[1999].bvid, "BV1999");
    }

    #[test]
    fn invalid_target_does_not_modify_archive_metadata() {
        let database = UserDatabase::open_in_memory().unwrap();
        assert!(database
            .write(|transaction| set(transaction, " ", 0, &[interval(1, 2)], 0, 1))
            .is_err());
        assert!(database.read(load_from).unwrap().is_empty());
    }

    #[test]
    fn target_options_keep_exact_page_id_and_duration_and_use_android_label_fallback() {
        let video: BiliVideoInfo = serde_json::from_value(json!({
            "bvid": " BVtest ", "title": "Video", "owner": "Owner", "cover": "", "cid": 9, "duration": 20,
            "pages": [
                {"cid": 9, "page": 1, "part": " Intro ", "duration": 20},
                {"cid": 10, "page": 2, "part": " ", "duration": 40},
                {"cid": 0, "page": 3, "part": "invalid", "duration": 5}
            ]
        })).unwrap();
        assert_eq!(
            target_options(&video),
            vec![
                BiliVideoSkipTargetOption {
                    bvid: "BVtest".into(),
                    cid: 9,
                    label: "Intro".into(),
                    duration_ms: 20000
                },
                BiliVideoSkipTargetOption {
                    bvid: "BVtest".into(),
                    cid: 10,
                    label: "P2".into(),
                    duration_ms: 40000
                },
            ]
        );
    }

    #[test]
    fn interval_changes_roll_back_with_the_archive_transaction() {
        let database = UserDatabase::open_in_memory().unwrap();
        let result: AppResult<()> = database.write(|transaction| {
            set(transaction, "BVtest", 1, &[interval(1, 2)], 0, 10)?;
            Err(AppError::Other("test rollback".into()))
        });
        assert!(result.is_err());
        assert!(database.read(load_from).unwrap().is_empty());
    }
}
