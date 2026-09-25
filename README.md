# SVNGUITool

SVN 提交查看工具。基于 **Rust + Slint** 构建的跨平台桌面 GUI，用于浏览 SVN 仓库/工作副本的提交历史、查看每次提交改动的文件与内容差异。

> 运行时依赖因平台而异：**macOS 使用 libsvn 动态库**访问 SVN；**Windows/Linux 通过 `svn` 命令行**执行 `log`/`diff`/`info` 等命令。

English version: [README.en.md](./README.en.md)

## 功能

- 连接 **SVN 仓库 URL** 或**本地工作副本路径**，自动识别仓库根与 HEAD 版本。
- **提交记录**列表：revision / 日期 / 作者 / 提交信息，倒序展示；点击查看该提交的改动文件列表和内容差异。
- **本次改动文件**列表：按提交展示新增/修改/删除的文件，自动加载第一个文件的 diff。
- **Diff 展示**：语法着色（新增/删除/头信息），支持查看文件全部历史中的任一次改动。
- **过滤面板**：关键词、作者、revision 区间、日期区间、条数（每批加载数量），支持应用过滤与刷新。
- **无限滚动**：列表滚动到末尾（或最后一条记录出现）时自动加载下一批更旧的提交，加载后保持当前窗口与滚动位置不变。
- 启动时自动连接：当前目录是工作副本（含 `.svn`）则直接使用；否则使用上次记忆的目标。

## 环境要求

- **macOS**：Homebrew 安装 `subversion`（含 `apr`、`apr-util`、`utf8proc`、`gettext`、`lz4`、`zlib` 等依赖）。程序通过 `libsvn` 动态库访问 SVN，**不需要** `svn` 命令行。构建前需先执行脚本生成 pkg-config 并设置环境（见下节）。
- **Linux / Windows**：`svn` 命令行客户端（含 `svn --version` 可用，需支持 `--xml` 输出）。Linux 安装 `subversion` 包，Windows 安装 TortoiseSVN 或 SlikSVN 并加入 PATH。
- **Rust 工具链**：stable（2021 edition），`cargo` 可用。Slint 组件的编译期生成依赖正常网络拉取 crates。

## 构建与运行

```bash
# macOS：先准备 libsvn 的 pkg-config 与链接环境（Homebrew 的 subversion 不带 .pc）
source scripts/macos-libsvn-env.sh

# 开发模式
cargo run

# 发布模式
cargo run --release
```

macOS 构建依赖 `subversion` crate（`subversion-sys` 通过 pkg-config 探测 `libsvn_*`）。脚本会生成 `target/svn-pc/svn_*-1.pc`，并导出 `PKG_CONFIG_PATH` / `LIBRARY_PATH`；如提示缺少 Homebrew 依赖，先执行 `brew install subversion apr apr-util utf8proc gettext lz4 zlib`。Linux/Windows 无需此脚本，直接 `cargo build` 即可。

启动后左上角输入仓库 URL 或本地工作副本路径，点击「连接」；也可点击「打开本地项目…」选择文件夹。

## 过滤说明

过滤面板各字段均为**与**关系，留空即不限制：

| 字段 | 含义 |
|---|---|
| 关键词 | 提交信息包含的文本（大小写不敏感） |
| 作者 | 提交作者（精确匹配） |
| rev 区间 | 形如 `100:200` 的 revision 范围 |
| 日期区间 | 形如 `2024-01-01:2024-01-31` 或 `2024-01-01:`（单侧）的日期范围 |
| 条数 | 每批从 svn 拉取的提交数量，默认 `500` |

「刷新」按当前条件重新加载；提交列表滚到底时若还有更多会自动追加下一批。

## 配置文件

首次连接成功后，程序把目标记忆到当前目录下的 `.svnguitool.json`（此文件已在 `.gitignore` 中）：

```json
{ "last_target": "https://example.com/svn/repo" }
```

## 测试

```bash
cargo test
```

单元测试覆盖配置读写、diff 着色、过滤解析、SVN XML 解析（Linux/Windows 下的命令行后端）等；集成测试会在临时目录创建真实 SVN 测试仓库。macOS 上集成测试走 libsvn 后端（`file://` 仓库），因此运行测试也需要 `svn`/`svnadmin`（创建仓库用），并先执行 `source scripts/macos-libsvn-env.sh`。

## 打包（CI）

`.github/workflows/build.yml` 在推送到 `master`/`main`（或打 `v*` 标签、手动触发）时，自动构建三个平台的可运行文件并作为 GitHub Actions artifact 上传：

| 平台 | 产物 |
|---|---|
| macOS (Apple Silicon) | `svnguitool-macos-arm64.tar.gz` |
| Linux (Ubuntu x86_64) | `svnguitool-linux-x86_64.tar.gz` |
| Windows (x86_64) | `svnguitool-windows-x86_64.zip` |

分发前请先将本仓库推送到 GitHub（`git push -u origin master`）。运行目标机器的依赖：**macOS 需 Homebrew `subversion` 动态库**（编译脚本同款依赖，`brew install subversion`）；**Linux/Windows 需 `svn` 命令行**。

## 项目结构

```
ui/app.slint       Slint 界面定义（组件、布局、回调）
src/main.rs        入口、连接/提交列表/加载更多/选中与 diff 展示
src/svn.rs          后端分派 + svn 命令行后端（Linux/Windows 用，cfg 门控）
src/svn/libsvn.rs    libsvn 后端（macOS 用）
scripts/macos-libsvn-env.sh  macOS 构建环境准备（生成 svn pkg-config）
src/diff.rs        diff 文本着色/分类
src/state.rs       过滤条件解析与匹配
src/config.rs      配置持久化 (.svnguitool.json)
build.rs           slint-build 编译 ui/app.slint
.github/workflows  三平台打包的 GitHub Actions
```

## 许可证

**GPLv3** — 见 [LICENSE](./LICENSE)。