//! macOS 专用后端：通过 libsvn（`subversion` crate）访问 SVN，替代 svn 命令行。
//!
//! 仅使用 `subversion` crate 的安全高层 API（`Context::log/info/diff_peg` 等），
//! 因此不引入 unsafe 代码。每个操作在自己的调用线程内创建独立的 Context
//! （libsvn 上下文非线程安全，操作本身由外部线程逐项调用）。

use std::sync::{Arc, Mutex};

use subversion::auth::{self, AuthBaton};
use subversion::client::{Context, DiffOptions, InfoOptions, LogOptions};
use subversion::io::{Stream, StreamBackend};
use subversion::{LogEntry as SvnLogEntry, NodeKind, Revision, RevisionRange, Revnum};

use super::{format_local_date, Change, LogEntry, RepoInfo, SvnError, Result};

fn to_err(e: subversion::Error<'static>) -> SvnError {
    SvnError::Command(e.full_message())
}

/// 创建非交互式 Context：只挂载凭据缓存类 provider（等效 `--non-interactive`），
/// 不启用任何交互提示。auth baton 挂载失败时不视为致命错误。
fn new_context() -> Result<Context> {
    let mut ctx = Context::new().map_err(to_err)?;
    if let Ok(baton) = AuthBaton::open(vec![
        auth::get_ssl_server_trust_file_provider(),
        auth::get_ssl_client_cert_file_provider(),
        auth::get_username_provider(),
    ]) {
        ctx.set_auth_owned(baton);
    }
    Ok(ctx)
}

/// 校验目标（URL 或本地工作副本路径）并解析出仓库根 URL 等信息。
pub fn connect(target: &str) -> Result<RepoInfo> {
    let mut ctx = new_context()?;
    // URL 用 Head（ra_local 对 Unspecified 会走到 wc_db 断言）；
    // WC 路径用 Working（等价 `svn info <WC>` 的基版本）。
    let revision = if subversion::path::is_url(target) {
        Revision::Head
    } else {
        Revision::Working
    };
    let options = InfoOptions {
        revision,
        ..Default::default()
    };
    let mut result: Option<(String, String, i64)> = None;
    ctx.info(target, &options, &|_path, info| {
        result = Some((
            info.url().to_string(),
            info.repos_root_url().to_string(),
            info.revision().as_i64(),
        ));
        Ok(())
    })
    .map_err(to_err)?;
    let (url, root, head_rev) =
        result.ok_or_else(|| SvnError::Parse("info 未返回结果".into()))?;
    Ok(RepoInfo { root, url, head_rev })
}

fn run_log(
    ctx: &mut Context,
    target: &str,
    ranges: &[RevisionRange],
    options: &LogOptions,
) -> Result<Vec<LogEntry>> {
    let mut out: Vec<LogEntry> = Vec::new();
    let mut failed: Option<SvnError> = None;
    ctx.log(&[target], ranges, options, &|entry| {
        match translate(entry) {
            Ok(e) => {
                out.push(e);
                Ok(())
            }
            Err(err) => {
                let msg = format!("{err}");
                failed = Some(err);
                Err(subversion::Error::from_message(&msg))
            }
        }
    })
    .map_err(|e| failed.unwrap_or_else(|| to_err(e)))?;
    Ok(out)
}

/// `svn log -v`：提交列表及其改动文件（供 Tab1）。
pub fn log_verbose(target: &str, limit: usize) -> Result<Vec<LogEntry>> {
    let limit = if limit == 0 { 500 } else { limit } as i32;
    let options = LogOptions {
        peg_revision: Revision::Head,
        limit: Some(limit),
        discover_changed_paths: true,
        ..Default::default()
    };
    let ranges = [RevisionRange::new(Revision::Head, Revision::Number(Revnum::from(1u64)))];
    let mut ctx = new_context()?;
    run_log(&mut ctx, target, &ranges, &options)
}

/// 分页：取比 `upto`（最新已加载的“最深”march revision）更旧的至多 `limit` 条。
/// 使用范围 `{upto-1}:1`，保证不含 `upto` 本身且不重复。`upto <= 1` 时直接返回空。
pub fn log_verbose_upto(target: &str, limit: usize, upto: i64) -> Result<Vec<LogEntry>> {
    if upto <= 1 {
        return Ok(Vec::new());
    }
    let limit = if limit == 0 { 500 } else { limit } as i32;
    let options = LogOptions {
        peg_revision: Revision::Head,
        limit: Some(limit),
        discover_changed_paths: true,
        ..Default::default()
    };
    let ranges = [RevisionRange::new(
        Revision::Number(Revnum::from((upto - 1) as u64)),
        Revision::Number(Revnum::from(1u64)),
    )];
    let mut ctx = new_context()?;
    run_log(&mut ctx, target, &ranges, &options)
}

/// 取 target(仓库内完整 URL) 在 rev 提交中的差异文本。
/// 使用 peg 限定 @rev（对应 `svn diff -c {rev} {url}@{rev}`），
/// 保证已删除的文件也能取到 diff。
pub fn diff(target_url: &str, rev: i64) -> Result<String> {
    let mut ctx = new_context()?;
    let peg = Revision::Number(Revnum::from(rev as u64));
    let start = Revision::Number(Revnum::from(rev.saturating_sub(1) as u64));
    let end = peg;

    let out = Arc::new(Mutex::new(Vec::new()));
    let err = Arc::new(Mutex::new(Vec::new()));
    let mut out_stream = Stream::from_backend(CaptureBackend { buf: out.clone() }).map_err(to_err)?;
    let mut err_stream = Stream::from_backend(CaptureBackend { buf: err.clone() }).map_err(to_err)?;

    ctx.diff_peg(
        target_url,
        &peg,
        &start,
        &end,
        None,
        &mut out_stream,
        &mut err_stream,
        &DiffOptions::default(),
    )
    .map_err(|e| {
        let stderr = String::from_utf8_lossy(&err.lock().unwrap()).into_owned();
        let msg = if stderr.trim().is_empty() {
            e.full_message()
        } else {
            stderr
        };
        SvnError::Command(msg)
    })?;

    let bytes = std::mem::take(&mut *out.lock().unwrap());
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

/// 把 libsvn 的日志条目翻译成应用的 LogEntry。
fn translate(entry: &SvnLogEntry) -> Result<LogEntry> {
    let revision = entry.revision().map(|r| r.as_i64()).unwrap_or(0);
    let mut e = LogEntry::new(revision);
    e.author = entry.author().unwrap_or_default().to_string();
    let raw_date = entry.date().unwrap_or_default().to_string();
    e.rfc3339 = raw_date.clone();
    e.date = format_local_date(&raw_date);
    e.msg = entry.message().unwrap_or_default().to_string();
    if let Some(paths) = entry.changed_paths() {
        let mut changes: Vec<Change> = paths
            .into_iter()
            .map(|(path, cp)| Change {
                action: cp.action,
                kind: match cp.node_kind {
                    NodeKind::File => "file".into(),
                    NodeKind::Dir => "dir".into(),
                    NodeKind::None | NodeKind::Unknown | NodeKind::Symlink => "?".into(),
                },
                path,
            })
            .collect();
        changes.sort_by(|a, b| a.path.cmp(&b.path));
        e.changes = changes;
    }
    Ok(e)
}

/// 把 svn 写入流的内容收集到共享缓冲里（diff 输出捕获用）。
struct CaptureBackend {
    buf: Arc<Mutex<Vec<u8>>>,
}

impl StreamBackend for CaptureBackend {
    fn write(&mut self, buf: &[u8]) -> std::result::Result<usize, subversion::Error<'static>> {
        self.buf.lock().expect("捕获缓冲锁").extend_from_slice(buf);
        Ok(buf.len())
    }

    fn close(&mut self) -> std::result::Result<(), subversion::Error<'static>> {
        Ok(())
    }
}