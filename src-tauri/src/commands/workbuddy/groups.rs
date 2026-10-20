//! Buddy 账号分组（对齐 Trae 账号分组能力，T：Buddy 分组）：
//! - P0 需求①：分组「定义」统一读共享 groups 表（shg_ 前缀，跨应用一份），
//!   create/update/remove 转发统一共享分组命令（旧命令名保留作薄封装）；
//! - 成员关系保持落 WB 账号记录的 `group_id` 字段（D1：随账号池 JSON 持久化，
//!   导入/导出天然携带，无需独立 membership 表）。

use tauri::State;

use crate::state::AppState;

use super::common::{load_pool, save_pool};

/// 分组列表（defs 读统一共享存储；count/uids 从账号池账号的 group_id 实时推导，
/// 供前端过滤 chips 与账号池分组筛选预览）。整池一次扫描构建映射（防 N+1）。
#[tauri::command]
pub fn workbuddy_groups_list(state: State<AppState>) -> Vec<crate::commands::accounts::GroupView> {
    let pool = load_pool(&state);
    let mut uids_by_gid: std::collections::HashMap<String, Vec<String>> = Default::default();
    for a in &pool.accounts {
        if !a.group_id.is_empty() {
            uids_by_gid.entry(a.group_id.clone()).or_default().push(a.id.clone());
        }
    }
    super::super::shared_groups::list_for_pool(&state, uids_by_gid)
}

#[tauri::command]
pub fn workbuddy_groups_create(state: State<AppState>, name: String, color: String) -> Result<String, String> {
    super::super::shared_groups::create_inner(&state, name, color)
}

#[tauri::command]
pub fn workbuddy_groups_update(
    state: State<AppState>,
    id: String,
    name: Option<String>,
    color: Option<String>,
    order: Option<i32>,
) -> Result<(), String> {
    super::super::shared_groups::update_inner(&state, id, name, color, order)
}

#[tauri::command]
pub fn workbuddy_groups_remove(
    state: State<AppState>,
    runtime: State<'_, std::sync::Mutex<Option<crate::commands::api_server::ApiServerRuntime>>>,
    id: String,
) -> Result<(), String> {
    // 转发统一删除：三池成员回落 + 清 api_pool/api_keys 引用 + 热重载
    // （原实现仅 WB 池回落，不清理 api_pool.wb_group_ids 引用——P0 收口）
    super::super::shared_groups::delete_inner(&state, runtime.inner(), &id)
}

/// 移动账号到分组（group_id=None 回落「未分组」）；user_id = 账号 id（wb- 前缀，与 save/remove 同键）。
/// 联动网关池热重载（对齐 qoder_account_move 惯例）：api_pool 的 wb_group_ids 按池内
/// group_id 过滤，服务运行时移动跨组不重载则网关仍按旧组过滤调度。
#[tauri::command]
pub fn workbuddy_account_move(
    state: State<AppState>,
    runtime: State<'_, std::sync::Mutex<Option<crate::commands::api_server::ApiServerRuntime>>>,
    user_id: String,
    group_id: Option<String>,
) -> Result<(), String> {
    // 读-改-写互斥：罩住 load→改→save 段防旧副本整文档覆盖。
    // 目标分组复检纳入同一临界区（持 wb_pool_lock 期间取 group_defs_lock，
    // 锁序池锁→defs锁）：校验与写入原子化——原校验在池锁外，校验后、写入前
    // 分组可被并发删除（delete_inner）→ 幽灵绑定（与 qoder_account_move 同构修复）
    let _guard = state.wb_pool_lock.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(gid) = group_id.as_deref().filter(|g| !g.is_empty()) {
        let _defs = state
            .group_defs_lock
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if !super::super::shared_groups::defs_load(&state).iter().any(|g| g.id == gid) {
            return Err(format!("目标分组不存在: {gid}"));
        }
        drop(_defs);
    }
    let mut pool = load_pool(&state);
    let acct = pool
        .accounts
        .iter_mut()
        .find(|a| a.id == user_id)
        .ok_or_else(|| format!("账号不存在: {user_id}"))?;
    acct.group_id = group_id.unwrap_or_default();
    let r = save_pool(&state, &pool);
    // 成员关系变化影响 org 展开索引与调度池分组筛选
    crate::api_server::org_index::invalidate(&state.data_dir);
    r?;
    // 先释放池锁再热重载（对齐 qoder_account_move 写法）：重载实现日后若引入
    // 池锁，持 wb_pool_lock 调 reload 即死锁；当前也是脆弱模式
    drop(_guard);
    crate::commands::api_server::reload_pools_if_running(&state, runtime.inner());
    Ok(())
}
