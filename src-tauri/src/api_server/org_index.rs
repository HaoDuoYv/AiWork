//! 分组/标签 → 成员 uid 展开索引（shared-groups-tags-design.md D4/D5）。
//!
//! 背景：API Key 的 `allowed_groups` / `allowed_tags` 绑定的是组织维度而非具体
//! 账号——成员变化须即时生效（不改 key/不重启），故在请求路径运行时实时展开，
//! 经 config_cache 缓存（TTL 5s 兜底 + 写路径显式失效），避免每请求查库。
//!
//! uid 命名空间不冲突是共享前提：Trae = 数字 user_id，Buddy = `wb-` 前缀，
//! Qoder = `qd-` 前缀。展开结果统一补池前缀（`trae:`/`buddy:`/`qoder:`）参与
//! `split_pool_tagged` 作用域判定，天然实现「一个 key 绑定跨三池的分组」。

use std::collections::{HashMap, HashSet};
use std::path::Path;

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Default)]
pub struct OrgIndex {
    /// 共享分组 id → 三池成员 uid 并集（原始 uid，不带池前缀）
    pub group_members: HashMap<String, Vec<String>>,
    /// 跨池账号 uid → 绑定的 tag id 列表
    pub account_tags: HashMap<String, Vec<String>>,
}

/// 读库构建索引。**可失败**：读库失败（瞬时锁冲突/库损坏等）向上传播，
/// 由 `get` 走旧快照兜底——静默回空索引会让全库账号在 TTL 窗口内不可取号。
fn load_index(data_dir: &Path) -> Result<OrgIndex, String> {
    let store = crate::store::db(data_dir);
    // 单次 with_conn 读齐全部源表：同一把连接锁持有期内完成，杜绝
    // 「分组读迁移前、标签读迁移后」的撕裂快照（此前分 4 次加锁，写路径
    // 可插在中间；跨进程 CLI 并发由 WAL 读快照 + TTL 5s 兜底）。
    // 表名均为本模块字面量常量（与 schema ROW_TABLES 白名单一致），无注入面。
    let (member_rows, pool_rows, tag_rows): (
        Vec<(String, String)>,
        Vec<(String, String)>,
        Vec<(String, String)>,
    ) = store
        .with_conn(|c| {
            let read_pairs = |sql: &str| -> rusqlite::Result<Vec<(String, String)>> {
                let mut stmt = c.prepare(sql)?;
                let rows = stmt
                    .query_map([], |r| {
                        Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            };
            let members = read_pairs("SELECT uid, group_id FROM group_members")?;
            let wb = read_pairs("SELECT pk, data FROM wb_accounts")?;
            let qoder = read_pairs("SELECT pk, data FROM qoder_accounts")?;
            let tags = read_pairs("SELECT pk, data FROM account_tags")?;
            Ok((members, [wb, qoder].concat(), tags))
        })?;
    let mut idx = OrgIndex::default();
    // Trae 成员：group_members 表
    for (uid, gid) in member_rows {
        idx.group_members.entry(gid).or_default().push(uid);
    }
    // Buddy / Qoder 成员：各自池账号 group_id 字段（行文档 data JSON）
    for (_pk, data) in pool_rows {
        let Ok(data) = serde_json::from_str::<serde_json::Value>(&data) else {
            continue;
        };
        let (Some(gid), Some(id)) = (
            data.get("group_id").and_then(|v| v.as_str()),
            data.get("id").and_then(|v| v.as_str()),
        ) else {
            continue;
        };
        if !gid.is_empty() {
            idx.group_members.entry(gid.to_string()).or_default().push(id.to_string());
        }
    }
    // 跨池账号 uid → 绑定的 tag id 列表
    idx.account_tags = tag_rows
        .into_iter()
        .filter_map(|(uid, data)| {
            let ids: Vec<String> = serde_json::from_str(&data).ok()?;
            Some((uid, ids))
        })
        .collect();
    Ok(idx)
}

/// 取索引（缓存优先；写路径 invalidate + TTL 5s 兜底）。
/// 读库失败：沿用旧快照（即使已过期）兜底并记日志，不缓存空索引；
/// 仅首启无任何快照时回退空索引（此时也无账号可展开，语义等价于空库）。
pub fn get(data_dir: &Path) -> OrgIndex {
    super::config_cache::get_or_load_fallible(
        data_dir,
        "org_index",
        || load_index(data_dir),
        || {
            crate::fs_utils::app_log(data_dir, "[org_index] 读库失败且无旧快照，回退空索引");
            OrgIndex::default()
        },
    )
}

/// 写路径显式失效（分组定义/成员/标签任何变更后调用，改动即时生效）
pub fn invalidate(data_dir: &Path) {
    super::config_cache::invalidate(data_dir, "org_index");
}

/// 池前缀归一：wb- → buddy:，qd- → qoder:，其余（数字 user_id）→ trae:
pub fn prefixed_uid(uid: &str) -> String {
    if uid.starts_with("wb-") {
        format!("buddy:{uid}")
    } else if uid.starts_with("qd-") {
        format!("qoder:{uid}")
    } else {
        format!("trae:{uid}")
    }
}

/// D5 交集展开：组与标签两维度间取交集，单一维度勾选时退化为该维度成员集，
/// 均空 = 不参与约束（返回空列表）。结果带池前缀、去重升序（确定性便于预览）。
pub fn expand(
    allowed_groups: &[String],
    allowed_tags: &[String],
    idx: &OrgIndex,
) -> Vec<String> {
    let in_groups: Option<HashSet<String>> = (!allowed_groups.is_empty()).then(|| {
        allowed_groups
            .iter()
            .filter_map(|g| idx.group_members.get(g))
            .flatten()
            .cloned()
            .collect()
    });
    let in_tags: Option<HashSet<String>> = (!allowed_tags.is_empty()).then(|| {
        idx.account_tags
            .iter()
            .filter(|(_, tags)| tags.iter().any(|t| allowed_tags.contains(t)))
            .map(|(uid, _)| uid.clone())
            .collect()
    });
    let effective: HashSet<String> = match (in_groups, in_tags) {
        (Some(g), Some(t)) => g.intersection(&t).cloned().collect(),
        (Some(g), None) => g,
        (None, Some(t)) => t,
        (None, None) => return Vec::new(),
    };
    let mut out: Vec<String> = effective.into_iter().map(|u| prefixed_uid(&u)).collect();
    out.sort();
    out.dedup();
    out
}

/// 调度池标签筛选展开（资源调度）：tag_ids 非空时返回「带任一所选标签」的 uid 集；
/// 空 = None（不限标签）。与既有分组筛选取交集（D5 池语义）。
pub fn tag_uid_set(
    tag_ids: &[String],
    account_tags: &HashMap<String, Vec<String>>,
) -> Option<HashSet<String>> {
    if tag_ids.is_empty() {
        return None;
    }
    Some(
        account_tags
            .iter()
            .filter(|(_, tags)| tags.iter().any(|t| tag_ids.contains(t)))
            .map(|(uid, _)| uid.clone())
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn idx(groups: &[(&str, &[&str])], tags: &[(&str, &[&str])]) -> OrgIndex {
        OrgIndex {
            group_members: groups
                .iter()
                .map(|(g, us)| (g.to_string(), us.iter().map(|s| s.to_string()).collect()))
                .collect(),
            account_tags: tags
                .iter()
                .map(|(u, ts)| (u.to_string(), ts.iter().map(|s| s.to_string()).collect()))
                .collect(),
        }
    }

    #[test]
    fn expand_intersection_and_single_dimension() {
        let i = idx(
            &[("shg_a", &["u1", "wb-2", "qd-3"])],
            &[("u1", &["tag_1"]), ("wb-2", &["tag_1", "tag_2"]), ("qd-3", &["tag_2"])],
        );
        let ga = &["shg_a".to_string()];
        // 组 ∩ 标签（D5）：仅 u1 与 wb-2 同时满足
        assert_eq!(expand(ga, &["tag_1".to_string()], &i), vec!["buddy:wb-2", "trae:u1"]);
        // 单维度退化
        assert_eq!(
            expand(ga, &[], &i),
            vec!["buddy:wb-2", "qoder:qd-3", "trae:u1"]
        );
        assert_eq!(expand(&[], &["tag_2".to_string()], &i), vec!["buddy:wb-2", "qoder:qd-3"]);
        // 均空 = 不约束
        assert!(expand(&[], &[], &i).is_empty());
        // 交集为空集（组内无人带标签）→ 空集（运行时该池无可取号账号）
        assert!(expand(ga, &["tag_x".to_string()], &i).is_empty());
    }

    #[test]
    fn prefixed_uid_by_namespace() {
        assert_eq!(prefixed_uid("12345"), "trae:12345");
        assert_eq!(prefixed_uid("wb-abc"), "buddy:wb-abc");
        assert_eq!(prefixed_uid("qd-xyz"), "qoder:qd-xyz");
    }

    #[test]
    fn tag_uid_set_none_when_empty() {
        let m = [("u1".to_string(), vec!["t1".to_string()])].into_iter().collect();
        assert!(tag_uid_set(&[], &m).is_none());
        let s = tag_uid_set(&["t1".to_string()], &m).unwrap();
        assert!(s.contains("u1"));
    }
}
