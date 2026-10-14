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

/// 在指定目录运行预期失败的 git-tc 命令，返回 stderr
fn tc_fail(dir: &TempDir, args: &[&str]) -> String {
    let output = Command::new(bin())
        .args(args)
        .current_dir(dir.path())
        .output()
        .unwrap();
    assert!(
        output.status.code().is_some_and(|c| c != 0),
        "命令预期失败但成功: {args:?}"
    );
    String::from_utf8_lossy(&output.stderr).to_string()
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

/// 以固定日期时间创建一个提交 (date/hm 来自 seed 提交的本地日期与偏移)
fn commit_at(dir: &TempDir, date: &str, offset: &str, hm: &str, msg: &str) {
    let iso = format!("{date}T{hm}:00{offset}");
    run(Command::new("git")
        .args(["commit", "-qm", msg])
        .env("GIT_AUTHOR_DATE", &iso)
        .env("GIT_COMMITTER_DATE", &iso)
        .current_dir(dir.path()));
}

/// 获取指定 rev 的 unix 提交时间戳
fn ct(dir: &TempDir, rev: &str) -> i64 {
    git(dir, &["show", "-s", "--format=%ct", rev])
        .trim()
        .parse()
        .unwrap()
}

#[test]
fn amend_on_merge_is_strictly_after_all_parents() {
    let dir = fresh_repo();
    tc(&dir, &["set", "--local", "09:00", "17:00"]);

    // merge base (真实时间，不参与断言；用其日期与偏移构造后续固定时间)
    fs::write(dir.path().join("a.txt"), "a\n").unwrap();
    git(&dir, &["add", "a.txt"]);
    git(&dir, &["commit", "-qm", "base"]);
    let seed = git(&dir, &["show", "-s", "--format=%cI", "HEAD"]);
    let date = seed[..10].to_string();
    let offset = seed[seed.len() - 6..].to_string();

    // 第二父 16:59 晚于第一父 09:00：amend 下界必须取所有父提交的最大值
    let base_branch = git(&dir, &["symbolic-ref", "--short", "HEAD"])
        .trim()
        .to_string();
    git(&dir, &["checkout", "-qb", "feature"]);
    fs::write(dir.path().join("f.txt"), "f\n").unwrap();
    git(&dir, &["add", "f.txt"]);
    commit_at(&dir, &date, &offset, "16:59", "F");

    git(&dir, &["checkout", "-q", &base_branch]);
    fs::write(dir.path().join("m.txt"), "m\n").unwrap();
    git(&dir, &["add", "m.txt"]);
    commit_at(&dir, &date, &offset, "09:01", "M");

    git(&dir, &["merge", "-q", "--no-ff", "feature", "-m", "merge"]);
    tc(&dir, &["amend"]);

    let amended = ct(&dir, "HEAD");
    assert!(
        amended > ct(&dir, "HEAD^2"),
        "amend 后 merge commit 时间必须晚于第二父 (较晚的父提交)"
    );
    assert!(
        amended > ct(&dir, "HEAD^"),
        "amend 后 merge commit 时间必须晚于第一父"
    );
}

#[test]
fn batch_amend_rewrites_range_with_window_and_monotonic_times() {
    let dir = fresh_repo();
    tc(&dir, &["set", "--local", "00:00", "04:00"]);

    // 种子提交 (真实时间)，仅用于取本地日期与时区偏移
    git(&dir, &["commit", "-qm", "seed", "--allow-empty"]);
    let seed = git(&dir, &["show", "-s", "--format=%cI", "HEAD"]);
    let date = seed[..10].to_string();
    let offset = seed[seed.len() - 6..].to_string();

    // base 固定在 18:00，使当日 00:00-04:00 窗口必然已过，结果必落在次日窗口
    fs::write(dir.path().join("base.txt"), "base\n").unwrap();
    git(&dir, &["add", "base.txt"]);
    commit_at(&dir, &date, &offset, "18:00", "base");
    let base = git(&dir, &["rev-parse", "HEAD"]).trim().to_string();
    let base_ct = ct(&dir, "HEAD");

    // 5 个待改写提交，固定在白天 10:00-10:40 (不在窗口内)
    for i in 0..5 {
        fs::write(dir.path().join(format!("f{i}.txt")), "x\n").unwrap();
        git(&dir, &["add", &format!("f{i}.txt")]);
        commit_at(
            &dir,
            &date,
            &offset,
            &format!("10:{:02}", i * 10),
            &format!("f{i}"),
        );
    }

    let tree_before = git(&dir, &["rev-parse", "HEAD^{tree}"]).trim().to_string();
    let range = format!("{base}..HEAD");
    tc(&dir, &["amend", &range]);

    // 5 个提交全部落在次日 (之后) 的 00:00-04:00 窗口内且同一天
    let mut cts = Vec::new();
    let mut first_day = String::new();
    for back in (0..5).rev() {
        let rev = format!("HEAD~{back}");
        let iso = git(&dir, &["show", "-s", "--format=%cI", &rev]);
        assert!(&iso[..10] > date.as_str(), "应顺延到之后的日期: {iso}");
        if first_day.is_empty() {
            first_day = iso[..10].to_string();
        }
        assert_eq!(&iso[..10], first_day, "应落在同一天窗口: {iso}");
        let (h, m) = time_of_day(&iso);
        assert!(h * 60 + m <= 4 * 60, "时间 {iso} 应在 00:00-04:00 窗口内");
        cts.push(ct(&dir, &rev));
    }

    // 严格递增且晚于 base
    assert!(cts[0] > base_ct, "首个改写提交必须晚于 base");
    for w in cts.windows(2) {
        assert!(w[0] < w[1], "改写后的时间必须严格递增: {cts:?}");
    }

    // 内容不变: 消息、树、提交数
    let log = git(&dir, &["log", "--format=%s", &range]);
    let subjects: Vec<&str> = log.lines().collect();
    assert_eq!(subjects, vec!["f4", "f3", "f2", "f1", "f0"]);
    assert_eq!(git(&dir, &["rev-parse", "HEAD^{tree}"]).trim(), tree_before);
}

#[test]
fn batch_amend_mid_range_preserves_descendant_commits() {
    let dir = fresh_repo();
    tc(&dir, &["set", "--local", "19:00", "23:30"]);

    git(&dir, &["commit", "-qm", "seed", "--allow-empty"]);
    let seed = git(&dir, &["show", "-s", "--format=%cI", "HEAD"]);
    let date = seed[..10].to_string();
    let offset = seed[seed.len() - 6..].to_string();

    fs::write(dir.path().join("base.txt"), "base\n").unwrap();
    git(&dir, &["add", "base.txt"]);
    commit_at(&dir, &date, &offset, "18:00", "base");

    for (i, hm) in ["19:30", "20:00", "20:30"].iter().enumerate() {
        fs::write(dir.path().join(format!("c{i}.txt")), "c\n").unwrap();
        git(&dir, &["add", &format!("c{i}.txt")]);
        commit_at(&dir, &date, &offset, hm, &format!("c{i}"));
    }
    fs::write(dir.path().join("d.txt"), "d\n").unwrap();
    git(&dir, &["add", "d.txt"]);
    commit_at(&dir, &date, &offset, "23:00", "d1");

    let d1_ct = ct(&dir, "HEAD");
    let base_ct = ct(&dir, "HEAD~4");
    let tree_before = git(&dir, &["rev-parse", "HEAD^{tree}"]).trim().to_string();
    let msgs_before = git(&dir, &["log", "--format=%s", "HEAD~4..HEAD"]);

    // 改写 base 之后的 3 个提交，d1 不在区间内
    tc(&dir, &["amend", "HEAD~4..HEAD~1"]);

    // 区间后的 d1 保持原时间，树与消息不变
    assert_eq!(ct(&dir, "HEAD"), d1_ct, "区间后的提交时间不应改变");
    assert_eq!(git(&dir, &["rev-parse", "HEAD^{tree}"]).trim(), tree_before);
    assert_eq!(
        git(&dir, &["log", "--format=%s", "HEAD~4..HEAD"]),
        msgs_before
    );

    // 区间内 3 个提交: 严格递增，落在 (base, d1) 与 19:00-23:30 窗口的交集
    let mut prev = base_ct;
    for back in (1..=3).rev() {
        let rev = format!("HEAD~{back}");
        let t = ct(&dir, &rev);
        assert!(t > prev, "必须严格递增");
        assert!(t < d1_ct, "必须早于区间后的提交");
        let iso = git(&dir, &["show", "-s", "--format=%cI", &rev]);
        assert_eq!(&iso[..10], date, "应落在同一天的窗口内: {iso}");
        let (h, m) = time_of_day(&iso);
        assert!(
            (19 * 60..=23 * 60 + 30).contains(&(h * 60 + m)),
            "时间 {iso} 应在 19:00-23:30 窗口内"
        );
        prev = t;
    }
}

#[test]
fn batch_amend_rejects_empty_range_and_non_head_ancestor() {
    let dir = fresh_repo();
    tc(&dir, &["set", "--local", "00:00", "04:00"]);

    fs::write(dir.path().join("a.txt"), "a\n").unwrap();
    git(&dir, &["add", "a.txt"]);
    git(&dir, &["commit", "-qm", "base"]);
    git(&dir, &["checkout", "-qb", "side"]);
    fs::write(dir.path().join("s.txt"), "s\n").unwrap();
    git(&dir, &["add", "s.txt"]);
    git(&dir, &["commit", "-qm", "side"]);
    git(&dir, &["checkout", "-q", "-"]);
    fs::write(dir.path().join("m.txt"), "m\n").unwrap();
    git(&dir, &["add", "m.txt"]);
    git(&dir, &["commit", "-qm", "main"]);

    let err = tc_fail(&dir, &["amend", "HEAD..HEAD"]);
    assert!(err.contains("没有提交"), "实际输出: {err}");
    let err = tc_fail(&dir, &["amend", "HEAD~1..side"]);
    assert!(err.contains("祖先"), "实际输出: {err}");
}
