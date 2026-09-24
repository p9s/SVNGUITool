#![deny(unsafe_code)]

mod config;
mod diff;
mod state;
mod svn;

use std::rc::Rc;
use std::sync::{Arc, Mutex};

use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel};

slint::include_modules!();

/// 全局日志兜底：丢弃日志，避免 icu4x 缺少 CJK 断词数据时刷屏。
struct DiscardLogger;

impl log::Log for DiscardLogger {
    fn enabled(&self, _metadata: &log::Metadata) -> bool {
        false
    }
    fn log(&self, _record: &log::Record) {}
    fn flush(&self) {}
}

fn install_logger() {
    let _ = log::set_boxed_logger(Box::new(DiscardLogger));
    log::set_max_level(log::LevelFilter::Trace);
}

/// 将 Vec 包装为 Slint 的模型。
fn modelrc<T: Clone + 'static>(items: Vec<T>) -> ModelRc<T> {
    Rc::new(VecModel::from(items)).into()
}

/// 跨线程共享的应用状态。
struct Shared {
    conn: Mutex<Option<svn::RepoInfo>>,
    commits: Mutex<Vec<svn::LogEntry>>,
    /// 已从 svn 拉取到的最深 revision（滚动加载基线；None=尚无数据）。
    commit_floor: Mutex<Option<i64>>,
    /// 提交列表代数：每次整体重载 +1，用于丢弃过期的“加载更多”结果。
    commits_gen: Mutex<u64>,
}

impl Shared {
    fn info(&self) -> Option<svn::RepoInfo> {
        self.conn.lock().unwrap().clone()
    }
}

fn set_status(ui: &MainWindow, msg: impl Into<SharedString>) {
    ui.set_status(msg.into());
}

fn set_status_weak(weak: &slint::Weak<MainWindow>, msg: impl Into<SharedString>) {
    let weak = weak.clone();
    let msg: SharedString = msg.into();
    let _ = slint::invoke_from_event_loop(move || {
        if let Some(ui) = weak.upgrade() {
            ui.set_status(msg.clone());
        }
    });
}

fn read_filter(ui: &MainWindow) -> state::FilterSpec {
    let limit = format!("{}", ui.get_filter_limit()).trim().parse().unwrap_or(500);
    state::FilterSpec {
        keyword: format!("{}", ui.get_filter_keyword()).trim().to_string(),
        author: format!("{}", ui.get_filter_author()).trim().to_string(),
        rev_min: state::parse_rev(&format!("{}", ui.get_filter_rev_min())),
        rev_max: state::parse_rev(&format!("{}", ui.get_filter_rev_max())),
        date_min: state::parse_date(&format!("{}", ui.get_filter_date_min())),
        date_max: state::parse_date(&format!("{}", ui.get_filter_date_max())),
        limit: if limit == 0 { 500 } else { limit },
    }
}

/// 连接仓库（后台线程），成功后会预填输入并加载提交记录。
fn connect_target(ui: &MainWindow, shared: &Arc<Shared>, target: &str) {
    let target = target.trim().to_string();
    if target.is_empty() {
        set_status(ui, "请输入 SVN 仓库 URL 或本地工作副本路径");
        return;
    }
    set_status(ui, format!("正在连接 {target} ..."));
    let shared = shared.clone();
    let weak = ui.as_weak();
    std::thread::spawn(move || {
        match svn::connect(&target) {
            Ok(info) => {
                *shared.conn.lock().unwrap() = Some(info.clone());
                let mut cfg = config::load();
                cfg.last_target = Some(target.clone());
                config::save(&cfg);
                let ok_target = target.clone();
                let status = format!("已连接: {} (HEAD r{})", info.url, info.head_rev);
                let shared2 = shared.clone();
                let _ = slint::invoke_from_event_loop(move || {
                    let ui = match weak.upgrade() {
                        Some(u) => u,
                        None => return,
                    };
                    ui.set_target_input(ok_target.into());
                    ui.set_current_target(target.clone().into());
                    set_status(&ui, status);
                    reload_commits(&ui, &shared2);
                });
            }
            Err(e) => set_status_weak(&weak, format!("连接失败: {e}")),
        }
    });
}

/// 重新加载提交列表（Tab1），并按当前过滤条件筛选。
fn reload_commits(ui: &MainWindow, shared: &Arc<Shared>) {
    let info = match shared.info() {
        Some(i) => i,
        None => {
            set_status(ui, "尚未连接仓库");
            return;
        }
    };
    let filter = read_filter(ui);
    let target = info.url.clone();
    let weak = ui.as_weak();
    let shared = shared.clone();
    set_status(ui, "正在加载提交记录 ...");
    std::thread::spawn(move || {
        let fetched = match svn::log_verbose(&target, filter.limit) {
            Ok(list) => list,
            Err(e) => {
                set_status_weak(&weak, format!("加载提交记录失败: {e}"));
                return;
            }
        };
        let total = fetched.len();
        let has_more = total >= filter.limit && total > 0;
        let floor = fetched.last().map(|e| e.revision);
        let entries: Vec<_> = fetched.into_iter().filter(|e| filter.matches(e)).collect();
        let count = entries.len();
        let mut items: Vec<CommitItem> = entries
            .iter()
            .map(|e| CommitItem {
                rev: e.revision.to_string().into(),
                author: e.author.clone().into(),
                date: e.date.clone().into(),
                msg: e.msg.clone().into(),
                loading: false,
            })
            .collect();
        // 列表末尾放“加载更多”哨兵行：被实例化（进入视野）时触发下一页加载
        if has_more {
            items.push(sentinel_commit());
        }
        let status = format!("提交记录 {count} 条 / 共 {total} 条");
        let shared2 = shared.clone();
        let _ = slint::invoke_from_event_loop(move || {
            let ui = match weak.upgrade() {
                Some(u) => u,
                None => return,
            };
            // 整体重载：旧“加载更多”结果一律作废
            *shared.commits_gen.lock().unwrap() += 1;
            *shared.commits.lock().unwrap() = entries.clone();
            *shared.commit_floor.lock().unwrap() = floor;
            ui.set_commit_model(modelrc(items));
            ui.set_has_more(has_more);
            ui.set_load_more_busy(false);
            set_status(&ui, status);
            if let Some(first) = entries.first() {
                select_commit(&ui, &shared2, first.revision);
            }
        });
    });
}

/// 提交列表滚动到底显示最后一条后，加载下一批（按 revision 分页，追加到模型尾部）。
fn load_more_commits(ui: &MainWindow, shared: &Arc<Shared>) {
    if ui.get_load_more_busy() || !ui.get_has_more() {
        return;
    }
    let info = match shared.info() {
        Some(i) => i,
        None => return,
    };
    let head = info.head_rev;
    let target = info.url.clone();
    let filter = read_filter(ui);
    let limit = filter.limit;
    let floor = *shared.commit_floor.lock().unwrap();
    let Some(mut floor) = floor else { return };
    let gen = *shared.commits_gen.lock().unwrap();

    let weak = ui.as_weak();
    let shared = shared.clone();
    ui.set_load_more_busy(true);
    set_status(ui, "正在加载更多提交记录 ...");
    std::thread::spawn(move || {
        let mut existing: std::collections::HashSet<i64> = shared
            .commits
            .lock()
            .unwrap()
            .iter()
            .map(|e| e.revision)
            .collect();
        let mut appended: Vec<svn::LogEntry> = Vec::new();
        let mut appended_count = 0usize;
        let mut has_more = true;

        for _ in 0..64 {
            if floor <= 1 {
                has_more = false;
                break;
            }
            let upto = (floor - 1).min(head);
            if upto < 1 {
                has_more = false;
                break;
            }
            let fetched = match svn::log_verbose_upto(&target, limit, upto) {
                Ok(list) => list,
                Err(e) => {
                    set_status_weak(&weak, format!("加载更多提交记录失败: {e}"));
                    has_more = false;
                    appended_count = 0;
                    break;
                }
            };
            let fetched_len = fetched.len();
            if fetched_len == 0 {
                has_more = false;
                break;
            }
            floor = fetched.last().unwrap().revision;
            let added: Vec<_> = fetched
                .into_iter()
                .filter(|e| filter.matches(e))
                .filter(|e| existing.insert(e.revision))
                .collect();
            appended_count += added.len();
            appended.extend(added);
            if appended_count > 0 || fetched_len < limit {
                break;
            }
        }

        let items: Vec<CommitItem> = appended
            .iter()
            .map(|e| CommitItem {
                rev: e.revision.to_string().into(),
                author: e.author.clone().into(),
                date: e.date.clone().into(),
                msg: e.msg.clone().into(),
                loading: false,
            })
            .collect();
        let _ = slint::invoke_from_event_loop(move || {
            let ui = match weak.upgrade() {
                Some(u) => u,
                None => return,
            };
            // 期间发生过整体重载 → 丢弃本次追加结果，仅复位 busy
            if *shared.commits_gen.lock().unwrap() != gen {
                ui.set_load_more_busy(false);
                return;
            }
            *shared.commit_floor.lock().unwrap() = Some(floor);
            shared.commits.lock().unwrap().extend(appended.iter().cloned());

            // 追加到同一模型实例（而非重建），保证当前窗口的提交与滚动位置不变。
            // 先摘掉末尾哨兵，再 push 新行；若仍有更多，把哨兵放回末尾。
            let current: ModelRc<CommitItem> = ui.get_commit_model();
            if let Some(m) = current.as_any().downcast_ref::<VecModel<CommitItem>>() {
                let count = m.row_count();
                if count > 0
                    && m.row_data(count - 1).is_some_and(|it| it.loading)
                {
                    m.remove(count - 1);
                }
                for it in items {
                    m.push(it);
                }
                if has_more {
                    m.push(sentinel_commit());
                }
            }
            let total = shared.commits.lock().unwrap().len();
            let status = if has_more {
                format!("已追加 {appended_count} 条，共 {total} 条（继续下拉加载更多）")
            } else {
                format!("已加载到最旧 revision，共 {total} 条")
            };
            ui.set_has_more(has_more);
            ui.set_load_more_busy(false);
            set_status(&ui, status);
        });
    });
}

/// commit-model 末尾的“加载更多”哨兵行。
fn sentinel_commit() -> CommitItem {
    CommitItem {
        rev: String::new().into(),
        author: String::new().into(),
        date: String::new().into(),
        msg: String::new().into(),
        loading: true,
    }
}

/// 选中某个提交：刷新改动文件列表，并自动展示第一个文件的 diff。
fn select_commit(ui: &MainWindow, shared: &Arc<Shared>, rev: i64) {
    ui.set_selected_rev(rev.to_string().into());
    let entry = shared
        .commits
        .lock()
        .unwrap()
        .iter()
        .find(|e| e.revision == rev)
        .cloned();
    let Some(entry) = entry else { return };
    let changes = entry.changes;
    ui.set_selected_change(-1);
    let items: Vec<ChangeItem> = changes
        .iter()
        .map(|c| ChangeItem {
            action: c.action.to_string().into(),
            path: c.path.clone().into(),
        })
        .collect();
    ui.set_change_model(modelrc(items));

    if let (Some(info), Some(first)) = (shared.info(), changes.first()) {
        load_diff(ui, &info, &first.path, rev);
    }
}

/// 将 diff::ColoredLine 转为 UI 的 DiffLine 模型。
fn diff_model(lines: Vec<diff::ColoredLine>) -> Vec<DiffLine> {
    lines
        .into_iter()
        .map(|l| {
            let color: slint::Brush = slint::Color::from_rgb_u8(l.rgb.0, l.rgb.1, l.rgb.2).into();
            DiffLine {
                color,
                bold: l.bold,
                italic: l.italic,
                text: l.text.into(),
            }
        })
        .collect()
}

/// 展示某个文件的 diff（后台线程执行 svn diff）。
fn load_diff(ui: &MainWindow, info: &svn::RepoInfo, path: &str, rev: i64) {
    let url = info.full_url(path);
    let disp_path = path.to_string();
    let weak = ui.as_weak();
    set_status(ui, format!("加载 r{rev}: {disp_path} ..."));
    std::thread::spawn(move || {
        let status = match svn::diff(&url, rev) {
            Ok(text) => {
                let lines = diff::render(&text);
                let rendered = if text.trim().is_empty() {
                    diff::render_msg("（无文本差异：可能是纯属性/二进制变更）", diff::GRAY)
                } else {
                    lines
                };
                let line_count = text.lines().count();
                let status = format!("r{rev}: {disp_path}（{line_count} 行）");
                let dup = weak.clone();
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(ui) = dup.upgrade() {
                        ui.set_diff_model(modelrc(diff_model(rendered)));
                        set_status(&ui, status);
                    }
                });
                return;
            }
            Err(e) => {
                let msg = format!("获取 r{rev} 的 diff 失败: {e}");
                let rendered = diff::render_msg(&msg, diff::RED);
                let dup = weak.clone();
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(ui) = dup.upgrade() {
                        ui.set_diff_model(modelrc(diff_model(rendered)));
                    }
                });
                msg
            }
        };
        set_status_weak(&weak, status);
    });
}

/// 重新加载提交列表（应用当前过滤条件）。
fn apply_filter(ui: &MainWindow, shared: &Arc<Shared>) {
    reload_commits(ui, shared);
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    install_logger();
    let ui = MainWindow::new()?;
    let shared = Arc::new(Shared {
        conn: Mutex::new(None),
        commits: Mutex::new(Vec::new()),
        commit_floor: Mutex::new(None),
        commits_gen: Mutex::new(0),
    });

    // ---- 状态栏初始提示 ----
    ui.set_status("SVNTool: 左侧输入 SVN 仓库 URL 或本地工作副本路径，点击「连接」".into());

    // ---- 绑定回调 ----
    {
        let shared = shared.clone();
        let weak = ui.as_weak();
        ui.on_connect(move |target: SharedString| {
            if let Some(ui) = weak.upgrade() {
                connect_target(&ui, &shared, &target);
            }
        });
    }
    {
        let shared = shared.clone();
        let weak = ui.as_weak();
        ui.on_open_local(move || {
            if let Some(ui) = weak.upgrade() {
                if let Some(dir) = rfd::FileDialog::new()
                    .set_title("选择本地 SVN 工作副本")
                    .pick_folder()
                {
                    let path = dir.to_string_lossy().into_owned();
                    ui.set_target_input(path.clone().into());
                    connect_target(&ui, &shared, &path);
                }
            }
        });
    }
    {
        let shared = shared.clone();
        let weak = ui.as_weak();
        ui.on_refresh(move || {
            if let Some(ui) = weak.upgrade() {
                apply_filter(&ui, &shared);
            }
        });
    }
    {
        let shared = shared.clone();
        let weak = ui.as_weak();
        ui.on_apply_filter(move || {
            if let Some(ui) = weak.upgrade() {
                apply_filter(&ui, &shared);
            }
        });
    }
    {
        let shared = shared.clone();
        let weak = ui.as_weak();
        ui.on_tab1_commit_selected(move |rev: SharedString| {
            if let Some(ui) = weak.upgrade() {
                if let Ok(rev) = format!("{}", rev).parse::<i64>() {
                    select_commit(&ui, &shared, rev);
                }
            }
        });
    }
    {
        let shared = shared.clone();
        let weak = ui.as_weak();
        ui.on_tab1_file_selected(move |idx: i32| {
            if let Some(ui) = weak.upgrade() {
                on_file_selected(&ui, &shared, idx);
            }
        });
    }
    {
        let shared = shared.clone();
        let weak = ui.as_weak();
        ui.on_load_more_commits(move || {
            if let Some(ui) = weak.upgrade() {
                load_more_commits(&ui, &shared);
            }
        });
    }

    // ---- 启动自动连接 ----
    let target = detect_start_target();
    if !target.is_empty() {
        let shared = shared.clone();
        let weak = ui.as_weak();
        let timer = slint::Timer::default();
        timer.start(slint::TimerMode::SingleShot, std::time::Duration::from_millis(1), move || {
            let target = target.clone();
            if let Some(ui) = weak.upgrade() {
                connect_target(&ui, &shared, &target);
            }
        });
        // 计时器必须存活到事件循环结束
        std::mem::forget(timer);
    }

    ui.run()?;
    Ok(())
}

/// 启动目标判定：当前目录是工作副本 -> 直接用；否则用配置文件记忆的目标。
fn detect_start_target() -> String {
    let cwd = std::env::current_dir();
    if let Ok(d) = &cwd {
        if d.join(".svn").is_dir() {
            return d.to_string_lossy().into_owned();
        }
    }
    config::load().last_target.unwrap_or_default()
}

fn on_file_selected(ui: &MainWindow, shared: &Arc<Shared>, idx: i32) {
    let rev: i64 = format!("{}", ui.get_selected_rev()).trim().parse().unwrap_or(0);
    let entry = shared
        .commits
        .lock()
        .unwrap()
        .iter()
        .find(|e| e.revision == rev)
        .cloned();
    let Some(entry) = entry else {
        set_status(ui, "未找到所选提交");
        return;
    };
    let Some(change) = entry.changes.get(idx as usize) else { return };
    ui.set_selected_change(idx);
    if let Some(info) = shared.info() {
        load_diff(ui, &info, &change.path, rev);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn commit_item(rev: i64) -> CommitItem {
        CommitItem {
            rev: rev.to_string().into(),
            author: "alice".into(),
            date: "2024-01-01 00:00".into(),
            msg: "msg".into(),
            loading: false,
        }
    }

    #[test]
    fn commit_model_downcast_push_appends_in_place() {
        let m: ModelRc<CommitItem> = Rc::new(VecModel::from(vec![commit_item(3), commit_item(2)])).into();
        let before = m.row_count();
        let v = m.as_any().downcast_ref::<VecModel<CommitItem>>().expect("应为 VecModel");
        v.push(commit_item(1));
        assert_eq!(m.row_count(), before + 1, "push 应就地追加同一模型实例");
        assert_eq!(m.row_data(before as usize).unwrap().rev, "1");
    }
}