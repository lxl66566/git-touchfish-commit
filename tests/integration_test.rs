use std::{
    fs,
    process::{Command, Output},
};

use tempfile::TempDir;

/// 获取被测二进制路径（cargo 自动注入 `CARGO_BIN_EXE_*` 环境变量）
fn bin() -> String {
    for name in &["CARGO_BIN_EXE_git-tc", "CARGO_BIN_EXE_git_tc"] {
        if let Ok(p) = std::env::var(name) {
            return p;
        }
    }
    panic!("CARGO_BIN_EXE_git-tc 未设置，请通过 `cargo test` 运行本测试");
}

/// 在指定目录运行 git 命令，返回 stdout；失败时 panic 并打印 stderr
fn git(dir: &TempDir, args: &[&str]) -> String {
    run(Command::new("git").args(args).current_dir(dir.path()))
}

/// 在指定目录运行 git-tc 命令
fn tc(dir: &TempDir, args: &[&str]) -> String {
    run(Command::new(bin()).args(args).current_dir(dir.path()))
}

fn run(cmd: &mut Command) -> String {
    let output: Output = cmd
        .output()
        .unwrap_or_else(|e| panic!("执行命令失败: {e:?}"));
    assert!(
        output.status.success(),
        "命令失败\n  stdout: {}\n  stderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    String::from_utf8_lossy(&output.stdout).to_string()
}

/// 在临时目录初始化一个 git 仓库
fn fresh_repo() -> TempDir {
    let dir = TempDir::new().expect("创建临时目录失败");
    git(&dir, &["init", "--quiet"]);
    git(&dir, &["config", "user.email", "test@example.com"]);
    git(&dir, &["config", "user.name", "Test User"]);
    git(&dir, &["config", "commit.gpgsign", "false"]);
    dir
}

/// 从 ISO 8601 字符串 (例如 `2024-01-15T14:30:00+08:00`) 提取 "HH:MM" 字段
fn time_of_day(iso: &str) -> (u32, u32) {
    // 0         1         2
    // 0123456789012345678901
    // 2024-01-15T14:30:00+08:00
    let (h, m) = iso[11..16].split_once(':').expect("ISO 时间格式异常");
    (h.parse().unwrap(), m.parse().unwrap())
}

#[test]
fn commit_sets_both_author_and_committer_date() {
    let dir = fresh_repo();

    // 配置时间区间到当前仓库 (--local)
    tc(&dir, &["set", "--local", "09:00", "17:00"]);

    // 准备一个文件并暂存
    fs::write(dir.path().join("hello.txt"), "hello world\n").unwrap();
    git(&dir, &["add", "hello.txt"]);

    // 用 git-tc 提交
    tc(&dir, &["-m", "first commit"]);

    // 读取 author date (%aI) 与 committer date (%cI)，均为严格 ISO 8601
    let author_date = git(&dir, &["show", "-s", "--format=%aI", "HEAD"])
        .trim()
        .to_string();
    let committer_date = git(&dir, &["show", "-s", "--format=%cI", "HEAD"])
        .trim()
        .to_string();

    assert!(
        !author_date.is_empty() && !committer_date.is_empty(),
        "author/committer date 不应为空"
    );

    // 关键断言 1：两者必须相等 —— 说明环境变量同时改写了两个 date
    assert_eq!(
        author_date, committer_date,
        "author date ({author_date}) 必须等于 committer date ({committer_date})"
    );

    // 关键断言 2：时间必须落在配置的 09:00-17:00 区间内
    let (h, m) = time_of_day(&author_date);
    let minutes = h * 60 + m;
    assert!(
        (9 * 60..=17 * 60).contains(&minutes),
        "提交时间 {h:02}:{m:02} 必须落在 09:00-17:00 区间内"
    );
}

#[test]
fn commit_date_is_strictly_after_previous() {
    let dir = fresh_repo();
    tc(&dir, &["set", "--local", "09:00", "17:00"]);

    // 第一次提交
    fs::write(dir.path().join("a.txt"), "a\n").unwrap();
    git(&dir, &["add", "a.txt"]);
    tc(&dir, &["-m", "commit 1"]);
    let first = git(&dir, &["show", "-s", "--format=%aI", "HEAD"])
        .trim()
        .to_string();

    // 第二次提交
    fs::write(dir.path().join("b.txt"), "b\n").unwrap();
    git(&dir, &["add", "b.txt"]);
    tc(&dir, &["-m", "commit 2"]);
    let second = git(&dir, &["show", "-s", "--format=%aI", "HEAD"])
        .trim()
        .to_string();

    // 第二次必须严格晚于第一次 (保证时间单调递增)
    assert!(
        second > first,
        "第二次提交时间 ({second}) 必须晚于第一次 ({first})"
    );

    // 两次都在 09:00-17:00 内
    for (label, iso) in [("第一次", &first), ("第二次", &second)] {
        let (h, m) = time_of_day(iso);
        let minutes = h * 60 + m;
        assert!(
            (9 * 60..=17 * 60).contains(&minutes),
            "{label}提交时间 {h:02}:{m:02} 必须落在 09:00-17:00 内"
        );
    }
}

#[test]
fn amend_changes_both_dates_and_preserves_author_identity() {
    let dir = fresh_repo();
    tc(&dir, &["set", "--local", "09:00", "17:00"]);

    // 先做一次提交
    fs::write(dir.path().join("c.txt"), "c\n").unwrap();
    git(&dir, &["add", "c.txt"]);
    tc(&dir, &["-m", "to be amended"]);

    let before_author_date = git(&dir, &["show", "-s", "--format=%aI", "HEAD"])
        .trim()
        .to_string();
    let before_committer_date = git(&dir, &["show", "-s", "--format=%cI", "HEAD"])
        .trim()
        .to_string();

    // amend：不带 --reset-author，应仅改写日期、不改作者身份
    tc(&dir, &["amend"]);

    let after_author_date = git(&dir, &["show", "-s", "--format=%aI", "HEAD"])
        .trim()
        .to_string();
    let after_committer_date = git(&dir, &["show", "-s", "--format=%cI", "HEAD"])
        .trim()
        .to_string();
    let author_email = git(&dir, &["show", "-s", "--format=%ae", "HEAD"])
        .trim()
        .to_string();
    let author_name = git(&dir, &["show", "-s", "--format=%an", "HEAD"])
        .trim()
        .to_string();

    // amend 后两个 date 仍然必须相等
    assert_eq!(
        after_author_date, after_committer_date,
        "amend 后 author date 与 committer date 应保持相等"
    );

    // amend 前后 date 应不同（理论上极小概率相同，但区间足够大可忽略）
    assert_ne!(
        before_author_date, after_author_date,
        "amend 应当改写 author date"
    );
    assert_ne!(
        before_committer_date, after_committer_date,
        "amend 应当改写 committer date"
    );

    // 作者身份应保持不变 (Test User / test@example.com)
    assert_eq!(author_email, "test@example.com");
    assert_eq!(author_name, "Test User");

    // 改写后的时间仍在窗口内
    let (h, m) = time_of_day(&after_author_date);
    let minutes = h * 60 + m;
    assert!(
        (9 * 60..=17 * 60).contains(&minutes),
        "amend 后时间 {h:02}:{m:02} 仍须落在 09:00-17:00 内"
    );
}

#[test]
fn amend_date_is_strictly_after_parent() {
    let dir = fresh_repo();
    tc(&dir, &["set", "--local", "09:00", "17:00"]);

    // 第一次提交（作为 amend 时的父提交 HEAD^）
    fs::write(dir.path().join("a.txt"), "a\n").unwrap();
    git(&dir, &["add", "a.txt"]);
    tc(&dir, &["-m", "commit 1"]);
    let parent = git(&dir, &["show", "-s", "--format=%aI", "HEAD"])
        .trim()
        .to_string();

    // 第二次提交，随后 amend 它
    fs::write(dir.path().join("b.txt"), "b\n").unwrap();
    git(&dir, &["add", "b.txt"]);
    tc(&dir, &["-m", "commit 2"]);
    tc(&dir, &["amend"]);

    let amended = git(&dir, &["show", "-s", "--format=%aI", "HEAD"])
        .trim()
        .to_string();
    assert!(
        amended > parent,
        "amend 后时间 ({amended}) 必须晚于父提交 ({parent})"
    );
}
