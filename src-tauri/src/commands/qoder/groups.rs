//! Qoder 账号分组（对齐 Buddy 账号分组能力，复用 accounts.rs GroupView / models::Group）：
//! - P0 需求①：分组「定义」统一读共享 groups 表（shg_ 前缀，跨应用一份），
//!   create/update/remove 转发统一共享分组命令（旧命令名保留作薄封装）；
//! - 成员关系保持落 Qoder 账号记录的 `group_id` 字段（serde(default)，随账号池 JSON
//!   持久化，导出/导入天然携带，无需独立 membership 表，D1）。

use tauri::State;

use crate::state::AppState;

use super::common::{load_pool, with_pool_mut};

/// 分组列表（defs 读统一共享存储；count/uids 从账号池账号的 group_id 实时推导，
/// 供前端过滤 chips 与编辑弹框下拉）。整池一次扫描构建映射（防 N+1）。
#[tauri::command]
pub fn qoder_groups_list(state: State<AppState>) -> Vec<crate::commands::accounts::GroupView> {
    let pool = load_pool(&state);
    let mut uids_by_gid: std::collections::HashMap<String, Vec<String>> = Default::default();
    for a in pool.iter() {
        if !a.group_id.is_empty() {
            uids_by_gid.entry(a.group_id.clone()).or_default().push(a.id.clone());
        }
    }
    super::super::shared_groups::list_for_pool(&state, uids_by_gid)
}

#[tauri::command]
pub fn qoder_groups_create(state: State<AppState>, name: String, color: String) -> Result<String, String> {
    // 转发统一创建（全局重名校验 + shg_ 新 ID；原 qoder_pool_lock 互斥由
    // defs 读改写的单连接串行化覆盖，对齐共享命令语义）
    super::super::shared_groups::create_inner(&state, name, color)
}

#[tauri::command]
pub fn qoder_groups_update(
    state: State<AppState>,
    id: String,
    name: Option<String>,
    color: Option<String>,
    order: Option<i32>,
) -> Result<(), String> {
    super::super::shared_groups::update_inner(&state, id, name, color, order)
}

#[tauri::command]
pub fn qoder_groups_remove(
    state: State<AppState>,
    runtime: State<'_, std::sync::Mutex<Option<crate::commands::api_server::ApiServerRuntime>>>,
    id: String,
) -> Result<(), String> {
    // 转发统一删除（顺序：先回落三池成员 → 再删定义 → 最后清 api_pool/api_keys
    // 引用 + 热重载；原 qoder 实现的审查结论已收敛进共享命令）
    super::super::shared_groups::delete_inner(&state, runtime.inner(), &id)
}

/// 移动账号到分组（group_id=None 回落「未分组」）；user_id = 账号 id（qd- 前缀，与 save/remove 同键）。
/// P2 审查修复：联动网关池热重载——api_pool 的 qoder_group_ids 按池内 group_id 过滤，
/// 服务运行时移动跨组不重载则网关仍按旧组过滤调度（红线 7 缺口，对齐 save/remove 惯例）
#[tauri::command]
pub fn qoder_account_move(
    state: State<AppState>,
    runtime: State<'_, std::sync::Mutex<Option<crate::commands::api_server::ApiServerRuntime>>>,
    user_id: String,
    group_id: Option<String>,
) -> Result<(), String> {
    with_pool_mut(&state, |accounts| {
        // 目标分组复检纳入池锁临界区（with_pool_mut 已持 qoder_pool_lock，此处
        // 取 group_defs_lock，锁序池锁→defs锁）：校验与写入原子化——原校验在
        // 池锁外，校验后、写入前分组可被并发删除（delete_inner）→ 幽灵绑定
        if let Some(gid) = group_id.as_deref().filter(|g| !g.is_empty()) {
            let _defs = state
                .group_defs_lock
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            if !super::super::shared_groups::defs_load(&state).iter().any(|g| g.id == gid) {
                return Err(format!("目标分组不存在: {gid}"));
            }
        }
        let acct = accounts
            .iter_mut()
            .find(|a| a.id == user_id)
            .ok_or_else(|| format!("账号不在池中: {user_id}"))?;
        acct.group_id = group_id.unwrap_or_default();
        Ok(())
    })?;
    crate::api_server::org_index::invalidate(&state.data_dir);
    crate::commands::api_server::reload_pools_if_running(&state, runtime.inner());
    Ok(())
}
