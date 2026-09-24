#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Added,
    Removed,
    Hunk,
    Header,
    NoNewline,
    Context,
}

pub fn classify(line: &str) -> Kind {
    if line.starts_with("@@ ") || line.starts_with("## ") {
        Kind::Hunk
    } else if line.starts_with('+') && !line.starts_with("+++ ") {
        Kind::Added
    } else if line.starts_with('-') && !line.starts_with("--- ") {
        Kind::Removed
    } else if line.starts_with("Index: ")
        || line.starts_with("====")
        || line.starts_with("--- ")
        || line.starts_with("+++ ")
        || line.starts_with("Property changes")
    {
        Kind::Header
    } else if line.starts_with('\\') {
        Kind::NoNewline
    } else {
        Kind::Context
    }
}

/// 每种行类型的默认前景色 (R, G, B)。
pub fn rgb(kind: Kind) -> (u8, u8, u8) {
    match kind {
        Kind::Added => (26, 127, 55),
        Kind::Removed => (207, 34, 46),
        Kind::Hunk => (87, 96, 106),
        Kind::Header => (87, 96, 106),
        Kind::NoNewline => (110, 119, 129),
        Kind::Context => (31, 35, 40),
    }
}

pub fn bold(kind: Kind) -> bool {
    matches!(kind, Kind::Hunk | Kind::Header)
}

pub fn italic(kind: Kind) -> bool {
    matches!(kind, Kind::NoNewline)
}

/// 一行 diff 的着色表示。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColoredLine {
    pub rgb: (u8, u8, u8),
    pub bold: bool,
    pub italic: bool,
    pub text: String,
}

/// 将 svn diff 文本渲染为着色行。
pub fn render(diff: &str) -> Vec<ColoredLine> {
    diff.lines()
        .map(|line| {
            let kind = classify(line);
            ColoredLine {
                rgb: rgb(kind),
                bold: bold(kind),
                italic: italic(kind),
                text: line.to_string(),
            }
        })
        .collect()
}

/// 单条提示行的着色渲染。
pub fn render_msg(msg: &str, rgb: (u8, u8, u8)) -> Vec<ColoredLine> {
    vec![ColoredLine {
        rgb,
        bold: false,
        italic: false,
        text: msg.to_string(),
    }]
}

pub const RED: (u8, u8, u8) = (207, 34, 46);
pub const GRAY: (u8, u8, u8) = (110, 119, 129);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_lines() {
        assert_eq!(classify("+added line"), Kind::Added);
        assert_eq!(classify("-removed line"), Kind::Removed);
        assert_eq!(classify("@@ -1,5 +1,6 @@"), Kind::Hunk);
        assert_eq!(classify("Index: src/main.rs"), Kind::Header);
        assert_eq!(classify("--- src/main.rs (revision 1)"), Kind::Header);
        assert_eq!(classify("+++ src/main.rs (revision 2)"), Kind::Header);
        assert_eq!(classify("======="), Kind::Header);
        assert_eq!(classify("Property changes on: f"), Kind::Header);
        assert_eq!(classify("\\ No newline at end of file"), Kind::NoNewline);
        assert_eq!(classify("     normal context"), Kind::Context);
        // +++ / --- 文件头不算增删
        assert_eq!(classify("+++ b/file (2)"), Kind::Header);
        assert_eq!(classify("--- a/file (1)"), Kind::Header);
    }

    #[test]
    fn colors_match_expected() {
        assert_eq!(rgb(Kind::Added), (26, 127, 55));
        assert_eq!(rgb(Kind::Removed), (207, 34, 46));
        assert_eq!(rgb(Kind::Context), (31, 35, 40));
    }

    #[test]
    fn render_styles_each_line() {
        let diff = "--- a/hello.txt (revision 2)\n+++ b/hello.txt (revision 3)\n@@ -1 +1 @@\n-hello\n+hello world\ncontext line\n";
        let lines = render(diff);
        assert_eq!(lines.len(), 6);

        assert_eq!(lines[0].rgb, rgb(Kind::Header));
        assert!(lines[0].bold);
        assert_eq!(lines[2].rgb, rgb(Kind::Hunk));
        assert!(lines[2].bold);

        assert_eq!(lines[3].text, "-hello");
        assert_eq!(lines[3].rgb, rgb(Kind::Removed));
        assert!(!lines[3].bold);

        assert_eq!(lines[4].text, "+hello world");
        assert_eq!(lines[4].rgb, rgb(Kind::Added));

        assert_eq!(lines[5].text, "context line");
        assert_eq!(lines[5].rgb, rgb(Kind::Context));
    }

    #[test]
    fn render_preserves_text_and_blank_lines() {
        let diff = "L1\n\nL3";
        let lines = render(diff);
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[1].text, "");
        assert_eq!(lines[0].text, "L1");
        assert_eq!(lines[2].text, "L3");
    }

    #[test]
    fn render_empty() {
        assert!(render("").is_empty());
    }

    #[test]
    fn render_no_newline_is_italic() {
        let lines = render("\\ No newline at end of file");
        assert_eq!(lines.len(), 1);
        assert!(lines[0].italic);
        assert_eq!(lines[0].rgb, rgb(Kind::NoNewline));
    }

    #[test]
    fn render_msg_helpers() {
        let err = render_msg("boom", RED);
        assert_eq!(err.len(), 1);
        assert_eq!(err[0].rgb, RED);
        assert_eq!(err[0].text, "boom");
    }
}