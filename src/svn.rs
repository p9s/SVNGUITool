use std::fmt;
use std::process::Command;

use quick_xml::events::Event;
use quick_xml::Reader;

const SVN_BIN: &str = "svn";

#[derive(Debug)]
pub enum SvnError {
    Io(std::io::Error),
    NonUtf8,
    Command(String),
    Parse(String),
}

impl fmt::Display for SvnError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SvnError::Io(e) => write!(f, "无法执行 svn 命令: {e}"),
            SvnError::NonUtf8 => write!(f, "svn 输出不是有效 UTF-8"),
            SvnError::Command(s) => write!(f, "{s}"),
            SvnError::Parse(s) => write!(f, "解析 svn 输出失败: {s}"),
        }
    }
}

impl std::error::Error for SvnError {}

pub type Result<T> = std::result::Result<T, SvnError>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    pub action: char,
    pub kind: String,
    pub path: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogEntry {
    pub revision: i64,
    pub author: String,
    pub date: String,
    pub rfc3339: String,
    pub msg: String,
    pub changes: Vec<Change>,
}

impl LogEntry {
    pub fn new(revision: i64) -> Self {
        LogEntry {
            revision,
            author: String::new(),
            date: String::new(),
            rfc3339: String::new(),
            msg: String::new(),
            changes: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoInfo {
    pub root: String,
    pub url: String,
    pub head_rev: i64,
}

impl RepoInfo {
    /// 拼接仓库内完整 URL（路径以 / 开头）。
    pub fn full_url(&self, path: &str) -> String {
        if path.is_empty() {
            self.root.clone()
        } else {
            format!("{}{}", self.root.trim_end_matches('/'), path)
        }
    }
}

fn run_svn(args: &[&str]) -> Result<String> {
    let out = Command::new(SVN_BIN)
        .arg("--non-interactive")
        .args(args)
        .output()
        .map_err(SvnError::Io)?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        let msg = err.trim().to_string();
        return Err(SvnError::Command(if msg.is_empty() {
            String::from_utf8_lossy(&out.stdout).trim().to_string()
        } else {
            msg
        }));
    }
    String::from_utf8(out.stdout).map_err(|_| SvnError::NonUtf8)
}

/// 校验目标（URL 或本地工作副本路径）并解析出仓库根 URL 等信息。
pub fn connect(target: &str) -> Result<RepoInfo> {
    let xml = run_svn(&["info", "--xml", target])?;
    parse_info(&xml)
}

/// `svn log -v`：提交列表及其改动文件（供 Tab1）。
pub fn log_verbose(target: &str, limit: usize) -> Result<Vec<LogEntry>> {
    let limit = if limit == 0 { 500 } else { limit };
    let xml = run_svn(&["log", "-v", "--xml", "-l", &limit.to_string(), target])?;
    Ok(parse_log(&xml))
}

/// 分页：取比 `upto`（最新已加载的“最深”march revision）更旧的至多 `limit` 条。
/// 使用 `-r {upto}:1` 范围，保证不含 `upto` 本身且不重复。`upto <= 1` 时直接返回空。
pub fn log_verbose_upto(target: &str, limit: usize, upto: i64) -> Result<Vec<LogEntry>> {
    if upto <= 1 {
        return Ok(Vec::new());
    }
    let limit = if limit == 0 { 500 } else { limit };
    let range = format!("{}:1", upto - 1);
    let xml = run_svn(&["log", "-v", "--xml", "-r", &range, "-l", &limit.to_string(), target])?;
    Ok(parse_log(&xml))
}

/// 取 target(仓库内完整 URL) 在 rev 提交中的差异文本。
/// 使用 peg 限定 @rev，保证已删除的文件也能取到 diff。
pub fn diff(target_url: &str, rev: i64) -> Result<String> {
    let peg = format!("{target_url}@{rev}");
    run_svn(&["diff", "-c", &rev.to_string(), &peg])
}

// ---------------- XML 解析 ----------------

fn event_name(name: quick_xml::name::QName) -> String {
    String::from_utf8_lossy(name.as_ref()).into_owned()
}

fn attr(buf: &quick_xml::events::BytesStart<'_>, key: &str) -> String {
    buf.attributes()
        .filter_map(|a| a.ok())
        .find(|a| a.key.as_ref() == key.as_bytes())
        .map(|a| String::from_utf8_lossy(a.value.as_ref()).into_owned())
        .unwrap_or_default()
}

fn text_of(bytes: &quick_xml::events::BytesText<'_>) -> String {
    bytes
        .unescape()
        .map(|c| c.into_owned())
        .unwrap_or_else(|_| String::from_utf8_lossy(bytes.as_ref()).into_owned())
}

fn format_local_date(iso: &str) -> String {
    use chrono::{DateTime, Local};
    match DateTime::parse_from_rfc3339(iso) {
        Ok(dt) => dt.with_timezone(&Local).format("%Y-%m-%d %H:%M").to_string(),
        Err(_) => iso.to_string(),
    }
}

/// 解析 `svn info --xml` 输出。
pub fn parse_info(xml: &str) -> Result<RepoInfo> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(false);

    let mut root = String::new();
    let mut url = String::new();
    let mut head_rev = 0i64;
    let mut text = String::new();

    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) => {
                if event_name(e.name()) == "commit" {
                    if let Ok(r) = attr(&e, "revision").parse::<i64>() {
                        head_rev = r;
                    }
                }
                text.clear();
            }
            Ok(Event::Text(t)) => text.push_str(&text_of(&t)),
            Ok(Event::End(e)) => {
                match event_name(e.name()).as_str() {
                    "root" => root = text.trim().to_string(),
                    "url" => url = text.trim().to_string(),
                    _ => {}
                }
                text.clear();
            }
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
    }
    if url.is_empty() {
        return Err(SvnError::Parse("未在输出中找到 <url>".into()));
    }
    Ok(RepoInfo { root, url, head_rev })
}

/// 解析 `svn log --xml` 输出。
pub fn parse_log(xml: &str) -> Vec<LogEntry> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(false);

    let mut entries: Vec<LogEntry> = Vec::new();
    let mut cur: Option<LogEntry> = None;
    let mut text = String::new();
    let mut pending_path: Option<(char, String)> = None;

    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) => {
                let name = event_name(e.name());
                if name == "logentry" {
                    let rev = attr(&e, "revision").parse::<i64>().unwrap_or(0);
                    cur = Some(LogEntry::new(rev));
                } else if name == "path" {
                    let action = attr(&e, "action").chars().next().unwrap_or('?');
                    let kind = attr(&e, "kind");
                    pending_path = Some((action, kind));
                }
                text.clear();
            }
            Ok(Event::Empty(e)) => {
                if event_name(e.name()) == "path" {
                    if let Some(c) = cur.as_mut() {
                        c.changes.push(Change {
                            action: attr(&e, "action").chars().next().unwrap_or('?'),
                            kind: attr(&e, "kind"),
                            path: String::new(),
                        });
                    }
                }
                text.clear();
            }
            Ok(Event::Text(t)) => text.push_str(&text_of(&t)),
            Ok(Event::End(e)) => {
                let name = event_name(e.name());
                match name.as_str() {
                    "logentry" => {
                        if let Some(c) = cur.take() {
                            entries.push(c);
                        }
                    }
                    "path" => {
                        if let Some((action, kind)) = pending_path.take() {
                            if let Some(c) = cur.as_mut() {
                                c.changes.push(Change {
                                    action,
                                    kind,
                                    path: text.trim().to_string(),
                                });
                            }
                        }
                    }
                    "author" => {
                        if let Some(c) = cur.as_mut() {
                            c.author = text.trim().to_string();
                        }
                    }
                    "date" => {
                        if let Some(c) = cur.as_mut() {
                            let raw = text.trim().to_string();
                            c.rfc3339 = raw.clone();
                            c.date = format_local_date(&raw);
                        }
                    }
                    "msg" => {
                        if let Some(c) = cur.as_mut() {
                            c.msg = text.trim().to_string();
                        }
                    }
                    _ => {}
                }
                text.clear();
            }
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
    }
    entries
}

// ---------------- 单元测试 ----------------

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_LOG: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<log>
<logentry revision="42">
<author>alice</author>
<date>2024-03-01T10:00:00.000000Z</date>
<msg>fix: &lt;bug&gt; in main</msg>
<paths>
<path kind="file" action="M" prop-mods="false" text-mods="true">/trunk/src/main.rs</path>
<path kind="file" action="A" text-mods="true">/trunk/src/helper.rs</path>
<path kind="dir" action="D">/trunk/old</path>
</paths>
</logentry>
<logentry revision="41">
<author>bob</author>
<date>2024-02-28T23:59:59.000000Z</date>
<msg></msg>
<paths>
<path kind="file" action="M">/trunk/README.md</path>
</paths>
</logentry>
</log>
"#;

    const SAMPLE_INFO: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<info>
<entry kind="dir" path="trunk" revision="42">
<url>http://svn.example.com/repo/trunk</url>
<relative-url>^/trunk</relative-url>
<repository>
<root>http://svn.example.com/repo</root>
<uuid>abc-def-123</uuid>
</repository>
<commit revision="42">
<author>alice</author>
<date>2024-03-01T10:00:00.000000Z</date>
</commit>
</entry>
</info>
"#;

    #[test]
    fn parse_log_basic() {
        let entries = parse_log(SAMPLE_LOG);
        assert_eq!(entries.len(), 2);

        let e = &entries[0];
        assert_eq!(e.revision, 42);
        assert_eq!(e.author, "alice");
        assert_eq!(e.rfc3339, "2024-03-01T10:00:00.000000Z");
        assert_eq!(e.msg, "fix: <bug> in main");
        assert_eq!(e.changes.len(), 3);
        assert_eq!(e.changes[0].path, "/trunk/src/main.rs");
        assert_eq!(e.changes[0].action, 'M');
        assert_eq!(e.changes[0].kind, "file");
        assert_eq!(e.changes[1].action, 'A');
        assert_eq!(e.changes[2].action, 'D');
        assert_eq!(e.changes[2].kind, "dir");

        let e2 = &entries[1];
        assert_eq!(e2.revision, 41);
        assert_eq!(e2.author, "bob");
        assert!(e2.msg.is_empty());
    }

    #[test]
    fn parse_log_empty_msg_and_html_escapes() {
        let entries = parse_log(SAMPLE_LOG);
        // 消息中的 < > 被 XML 转义，解析后应还原
        assert_eq!(entries[0].msg, "fix: <bug> in main");
    }

    #[test]
    fn parse_log_empty_input() {
        assert!(parse_log("").is_empty());
        assert!(parse_log("<log></log>").is_empty());
    }

    #[test]
    fn parse_info_basic() {
        let info = parse_info(SAMPLE_INFO).unwrap();
        assert_eq!(info.root, "http://svn.example.com/repo");
        assert_eq!(info.url, "http://svn.example.com/repo/trunk");
        assert_eq!(info.head_rev, 42);
    }

    #[test]
    fn parse_info_missing_url_is_error() {
        assert!(parse_info("<info></info>").is_err());
    }

    #[test]
    fn full_url_join() {
        let info = RepoInfo {
            root: "http://s/repo".into(),
            url: "http://s/repo/trunk".into(),
            head_rev: 5,
        };
        assert_eq!(info.full_url(""), "http://s/repo");
        assert_eq!(info.full_url("/trunk/src/main.rs"), "http://s/repo/trunk/src/main.rs");
    }

    #[test]
    fn error_display_has_message() {
        let e = SvnError::Command("E200009 无法显示".into());
        assert!(e.to_string().contains("E200009"));
    }
}

// ---------------- 集成测试（真实 svn, 用 /tmp 临时仓库） ----------------

#[cfg(test)]
mod integration_tests {
    use super::*;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static COUNTER: AtomicUsize = AtomicUsize::new(0);

    struct TestRepo {
        dir: PathBuf,
        url: String,
    }

    impl TestRepo {
        fn new(name: &str) -> Self {
            if Command::new("svnadmin").arg("--version").output().is_err() {
                panic!("未找到 svnadmin，无法运行集成测试");
            }
            let n = COUNTER.fetch_add(1, Ordering::SeqCst);
            let base = std::env::temp_dir().join(format!("svntool_test_{}_{}_{}", name, std::process::id(), n));
            let _ = fs::remove_dir_all(&base);
            fs::create_dir_all(&base).unwrap();

            // 创建一个标准布局仓库
            let repo = base.join("repo");
            assert!(Command::new("svnadmin").arg("create").arg(&repo).status().unwrap().success());
            let url = format!("file://{}", repo.display());

            // 导入初始目录结构（trunk/branches/tags）
            let layout = base.join("layout");
            fs::create_dir_all(layout.join("trunk")).unwrap();
            fs::create_dir_all(layout.join("branches")).unwrap();
            fs::create_dir_all(layout.join("tags")).unwrap();
            assert!(Command::new("svn")
                .args(["import", "-q", "-m", "init layout"])
                .arg(&layout)
                .arg(format!("{url}/"))
                .status()
                .unwrap()
                .success());

            // 提交第一条内容
            let wc = base.join("wc");
            assert!(Command::new("svn")
                .args(["checkout", "-q", format!("{url}/trunk").as_str()])
                .arg(&wc)
                .status()
                .unwrap()
                .success());
            fs::write(wc.join("hello.txt"), "hello\n").unwrap();
            Self::svn_add(&wc, &["hello.txt"]);
            Self::svn_commit(&wc, "add hello");
            fs::write(wc.join("hello.txt"), "hello world\n").unwrap();
            fs::write(wc.join("bye.py"), "print('bye')\n").unwrap();
            Self::svn_add(&wc, &["bye.py"]);
            Self::svn_commit(&wc, "modify hello, add bye");

            TestRepo { dir: base, url }
        }

        fn svn_add(wc: &Path, paths: &[&str]) {
            let status = Command::new("svn")
                .current_dir(wc)
                .arg("add")
                .args(paths)
                .status()
                .unwrap();
            assert!(status.success());
        }

        fn svn_commit(wc: &Path, msg: &str) {
            let status = Command::new("svn")
                .current_dir(wc)
                .args(["commit", "-q", "-m", msg, "."])
                .status()
                .unwrap();
            assert!(status.success());
        }
    }

    impl Drop for TestRepo {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.dir);
        }
    }

    #[test]
    fn integration_connect_and_url() {
        let repo = TestRepo::new("connect");
        let info = connect(&repo.url).expect("应能连接 file:// 仓库");
        assert_eq!(info.root, repo.url);
        assert!(!info.url.is_empty());
        assert!(info.head_rev >= 3);
    }

    #[test]
    fn integration_connect_from_local_wc() {
        // 通过本地工作副本路径连接：应解析出远程仓库根 URL（"打开本地项目"功能的后端链路）
        let repo = TestRepo::new("wc");
        let wc_path = repo.dir.join("wc");
        let info = connect(wc_path.to_str().unwrap()).expect("应能从本地工作副本连接");
        assert_eq!(info.root, repo.url);
        assert_eq!(info.url, format!("{}/trunk", repo.url));

        // 经工作副本解析出的 root 也能取到远程 diff / 记录
        let d = diff(&info.full_url("/trunk/hello.txt"), 3).unwrap();
        assert!(d.contains("+hello world"));
        let entries = log_verbose(&info.root, 100).unwrap();
        assert_eq!(entries[0].revision, 3);
    }

    #[test]
    fn integration_log_has_changes() {
        let repo = TestRepo::new("log");
        let entries = log_verbose(&format!("{}/trunk", repo.url), 100).unwrap();
        // r3: modify hello + add bye ; r2: init? 布局初始化在 r1
        assert!(!entries.is_empty());
        let newest = &entries[0];
        assert_eq!(newest.revision, 3);
        assert_eq!(newest.changes.len(), 2);
        let paths: Vec<&str> = newest.changes.iter().map(|c| c.path.as_str()).collect();
        assert!(paths.contains(&"/trunk/hello.txt"));
        assert!(paths.contains(&"/trunk/bye.py"));

        // -v 输出应包含 author/date/rfc3339（非空）
        assert!(!newest.author.is_empty());
        assert!(!newest.rfc3339.is_empty());
        assert!(!newest.date.is_empty());
    }

    #[test]
    fn integration_diff_shows_add_and_modify() {
        let repo = TestRepo::new("diff");

        // r3 中 hello.txt 为修改：应出现 -hello +hello world
        let hello_url = format!("{}/trunk/hello.txt", repo.url);
        let d = diff(&hello_url, 3).unwrap();
        assert!(d.contains("+hello world"));
        assert!(d.contains("-hello"));

        // r2 中 hello.txt 为新增：应出现全部新增内容
        let d2 = diff(&hello_url, 2).unwrap();
        assert!(d2.contains("+hello"));
    }

    #[test]
    fn integration_error_on_bad_url() {
        let url = format!("file://{}/nonexistent_repo_xyz", std::env::temp_dir().display());
        assert!(connect(&url).is_err());
    }

    #[test]
    fn integration_log_upto_paginates_without_dup_or_gap() {
        let repo = TestRepo::new("page");
        let wc = repo.dir.join("wc");
        for i in 2..=7 {
            fs::write(wc.join("hello.txt"), format!("v{i}\n")).unwrap();
            TestRepo::svn_commit(&wc, &format!("commit {i}"));
        }
        // r1 布局, r2 add hello, r3 modify+bye, r4..r9 逐次修改
        let target = format!("{}/trunk", repo.url);

        let page1 = log_verbose(&target, 3).unwrap();
        assert_eq!(page1.len(), 3);
        let floor = page1.last().unwrap().revision;

        let page2 = log_verbose_upto(&target, 3, floor).unwrap();
        assert_eq!(page2.len(), 3);

        let all: Vec<i64> = page1.iter().chain(page2.iter()).map(|e| e.revision).collect();
        let set: std::collections::HashSet<i64> = all.iter().cloned().collect();
        assert_eq!(all.len(), set.len(), "分页不应产生重复 revision");
        assert!(all.windows(2).all(|w| w[0] > w[1]), "整体应保持倒序");

        let page3 = log_verbose_upto(&target, 10, page2.last().unwrap().revision).unwrap();
        assert_eq!(page3.last().unwrap().revision, 1, "分页应到达最旧 revision r1");

        assert!(log_verbose_upto(&target, 10, 1).unwrap().is_empty(), "upto=1 应返回空");
        let only_oldest = log_verbose_upto(&target, 10, 2).unwrap();
        assert!(only_oldest.iter().all(|e| e.revision == 1), "upto=2 应只剩 r1");
    }
}