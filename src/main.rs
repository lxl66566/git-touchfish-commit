use std::{
    collections::{BTreeSet, HashMap, HashSet},
    io::Write,
    process::{Command, Stdio},
};

use chrono::{DateTime, Duration, Local, NaiveDate, NaiveDateTime, NaiveTime, TimeZone};
use rand::RngExt;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

/// (commit hash, parent hashes) 列表，拓扑序 (父提交在子提交之前)
type CommitList = Vec<(String, Vec<String>)>;
/// (下界, 上界)，均可为 None 表示无界
type TimeBounds = (Option<DateTime<Local>>, Option<DateTime<Local>>);

const CONFIG_KEY_START: &str = "touchfish.start-time";
const CONFIG_KEY_END: &str = "touchfish.end-time";
const DEFAULT_START: &str = "00:00";
const DEFAULT_END: &str = "04:00";

/// 打印用法信息
fn print_usage() {
    eprintln!(
        "Usage:\n  \
         git-tc set [--global|--local] <start> <end>   设置时间区间 (HH:MM)，默认 --global\n  \
         git-tc show                                    查看当前时间区间\n  \
         git-tc amend [args...]                         用随机时间修改最后一次提交\n  \
         git-tc amend <aaa>..<bbb>                      批量用随机时间改写区间内所有提交\n  \
         git-tc <args...>                               等同 git commit <args...>，使用随机时间\n\n\
         时间区间支持跨天 (start > end 表示次日 end)，例如 22:00 02:00"
    );
}

fn main() -> Result<()> {
    eprintln!("{} v{}", env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION"));
    let cli: Vec<String> = std::env::args().skip(1).collect();

    if cli.is_empty() {
        print_usage();
        return Ok(());
    }

    match cli[0].as_str() {
        "set" => handle_set(&cli[1..])?,
        "show" => show_time_range()?,
        "amend" => handle_amend(&cli[1..])?,
        _ => run_commit(&cli)?,
    }

    Ok(())
}

// --- 子命令实现 ---

fn handle_set(args: &[String]) -> Result<()> {
    let (global, rest) = match args.first().map(String::as_str) {
        Some("--global") => (true, &args[1..]),
        Some("--local") => (false, &args[1..]),
        _ => (true, args),
    };

    if rest.len() != 2 {
        eprintln!("用法: git-tc set [--global|--local] <start> <end>，时间应为 HH:MM 格式");
        eprintln!("默认写入 --global 配置使所有仓库共享；使用 --local 仅写入当前仓库");
        return Ok(());
    }

    let start = parse_time(&rest[0])?;
    let end = parse_time(&rest[1])?;

    if start == end {
        return Err("开始时间与结束时间不能相同".into());
    }

    let scope = if global {
        "--global"
    } else {
        "--local"
    };
    for (key, val) in [(CONFIG_KEY_START, &rest[0]), (CONFIG_KEY_END, &rest[1])] {
        let status = Command::new("git")
            .args(["config", scope, key, val])
            .status()?;
        if !status.success() {
            return Err(format!("git config {scope} {key} 失败").into());
        }
    }

    eprintln!("时间区间已设置 ({scope}): {} - {}", rest[0], rest[1]);
    Ok(())
}

fn show_time_range() -> Result<()> {
    let start = read_git_config(CONFIG_KEY_START)?;
    let end = read_git_config(CONFIG_KEY_END)?;
    if let (Some(s), Some(e)) = (start, end) {
        eprintln!("当前时间区间: {s} - {e}");
    } else {
        eprintln!("未显式配置，使用默认值: {DEFAULT_START} - {DEFAULT_END}");
        eprintln!("提示: 使用 `git-tc set <start> <end>` 进行配置");
    }
    Ok(())
}

fn run_commit(args: &[String]) -> Result<()> {
    let random_dt = generate_random_commit_time(get_commit_time("HEAD")?)?;
    let formatted = random_dt.to_rfc3339();
    eprintln!("正在使用随机时间 {formatted} 执行 git commit...");

    let status = Command::new("git")
        .arg("commit")
        .args(args)
        .env("GIT_AUTHOR_DATE", &formatted)
        .env("GIT_COMMITTER_DATE", &formatted)
        .status()?;

    if !status.success() {
        return Err("git commit 失败".into());
    }
    eprintln!("git commit 成功。");
    Ok(())
}

fn amend_commit_time(args: &[String]) -> Result<()> {
    // 关键背景：git 在 `--amend` 时若没有 `--reset-author`，会原样保留 author
    // date， 完全忽略 GIT_AUTHOR_DATE 环境变量。因此必须加 `--reset-author`
    // 才能改写 author date。 但 `--reset-author` 会同时重置作者身份
    // (name/email)。为避免覆盖原作者信息， 这里先把原作者的 name/email
    // 读出来，通过 GIT_AUTHOR_NAME/EMAIL 环境变量回填。
    let orig_name = capture_git(&["show", "-s", "--format=%an", "HEAD"])?.unwrap_or_default();
    let orig_email = capture_git(&["show", "-s", "--format=%ae", "HEAD"])?.unwrap_or_default();

    // amend 会用新提交替换 HEAD，为保证时间仍单调递增，时间下界应取被替换
    // 提交全部父提交时间的最大值 (merge commit 有多个父提交)，而不是 HEAD
    // 本身（HEAD 的旧时间无需作为下界）。若 HEAD 是根提交，则无下界。
    let random_dt = generate_random_commit_time(max_parent_time()?)?;
    let formatted = random_dt.to_rfc3339();
    eprintln!("正在使用随机时间 {formatted} 修改最后一次 commit...");

    let status = Command::new("git")
        .arg("commit")
        .arg("--amend")
        .arg("--no-edit")
        .arg("--reset-author")
        .args(args)
        .env("GIT_AUTHOR_NAME", &orig_name)
        .env("GIT_AUTHOR_EMAIL", &orig_email)
        .env("GIT_AUTHOR_DATE", &formatted)
        .env("GIT_COMMITTER_DATE", &formatted)
        .status()?;

    if !status.success() {
        return Err("amend 失败".into());
    }
    eprintln!("amend 成功。");
    Ok(())
}

fn handle_amend(args: &[String]) -> Result<()> {
    // 首参数形如 aaa..bbb 时进入批量改写；其余情况保持单提交 amend 的参数透传。
    // 只检查首参数，避免误吞用户传给 git commit 的普通参数
    if let Some((start, end)) = args.first().and_then(|a| a.split_once("..")) {
        if args.len() > 1 {
            eprintln!("用法: git-tc amend <aaa>..<bbb> 不接受额外参数");
            return Ok(());
        }
        if end.starts_with('.') {
            eprintln!("仅支持两点区间 <aaa>..<bbb>，不支持三点区间");
            return Ok(());
        }
        let start = if start.is_empty() {
            "HEAD"
        } else {
            start
        };
        let end = if end.is_empty() {
            "HEAD"
        } else {
            end
        };
        return batch_amend(start, end);
    }
    amend_commit_time(args)
}

/// 批量改写 aaa..bbb 区间内全部提交的 author/committer 时间。
///
/// 时间点在 (lo, hi) 与每日时间窗口的交集内一次性采样 (见 compute_random_times)，
/// 再用 hash-object 重建整条链：区间内提交替换时间，区间后提交仅重映射父提交、
/// 时间不变。不逐个 `commit --amend`，那样新提交只能作为前一个的下界约束，
/// 时间会向窗口末尾聚集。
fn batch_amend(start_rev: &str, end_rev: &str) -> Result<()> {
    let resolve =
        |rev: &str| capture_git_strict(&["rev-parse", "--verify", &format!("{rev}^{{commit}}")]);
    let start = resolve(start_rev)?;
    let end = resolve(end_rev)?;
    let head = capture_git(&["rev-parse", "HEAD"])?.ok_or("仓库没有 HEAD 提交")?;

    if !Command::new("git")
        .args(["merge-base", "--is-ancestor", &end, &head])
        .status()?
        .success()
    {
        return Err(format!("{end_rev} 不是 HEAD 的祖先，仅支持改写 HEAD 上的历史").into());
    }

    let range = rev_list_with_parents(&format!("{start}..{end}"))?;
    if range.is_empty() {
        return Err(format!("区间 {start_rev}..{end_rev} 内没有提交").into());
    }
    let descendants = rev_list_with_parents(&format!("{end}..{head}"))?;
    let range_set: HashSet<&str> = range.iter().map(|(h, _)| h.as_str()).collect();

    let (lo, hi) = batch_time_bounds(&range, &range_set, &descendants)?;
    let (start_time, end_time) = load_time_range()?;
    let times = compute_random_times(start_time, end_time, lo, hi, range.len(), Local::now())?;

    eprintln!(
        "批量改写 {} 个提交，下界: {}，上界: {}",
        range.len(),
        lo.map_or_else(|| "<无>".into(), |t| t.to_string()),
        hi.map_or_else(|| "<无，向后顺延>".into(), |t| t.to_string()),
    );

    let (map, signed_dropped) = rebuild_chain(&range, &descendants, &times)?;
    if signed_dropped > 0 {
        eprintln!("警告: 改写导致 {signed_dropped} 个提交的 GPG 签名失效并被丢弃");
    }
    warn_diverged_branches(&map)?;

    let new_tip = map.get(&head).ok_or("内部错误: HEAD 未被重建")?.clone();
    let status = Command::new("git")
        .args(["update-ref", "HEAD", &new_tip, &head])
        .status()?;
    if !status.success() {
        return Err("git update-ref 失败".into());
    }
    if !descendants.is_empty() {
        eprintln!("已重建 {} 个区间后的提交 (时间保持不变)", descendants.len());
    }
    eprintln!("新 HEAD: {new_tip}，原 HEAD: {head}，可用 `git reset {head}` 回退");
    Ok(())
}

/// 计算批量改写的时间边界：
/// lo = 区间内提交的全部区间外父提交 (含 merge 的旁路父) 的最大时间；
/// hi = 区间提交的最早未改写子提交时间，无子提交 (区间终点即 tip) 时为 None。
/// 新时间整体落在 (lo, hi) 内才能同时保持与前后两段严格单调。
fn batch_time_bounds(
    range: &CommitList,
    range_set: &HashSet<&str>,
    descendants: &CommitList,
) -> Result<TimeBounds> {
    let mut lo = None;
    for (_, parents) in range {
        for p in parents {
            if !range_set.contains(p.as_str())
                && let Some(t) = get_commit_time(p)?
                && lo.is_none_or(|m| t > m)
            {
                lo = Some(t);
            }
        }
    }
    let mut hi = None;
    for (d, parents) in descendants {
        if parents.iter().any(|p| range_set.contains(p.as_str()))
            && let Some(t) = get_commit_time(d)?
            && hi.is_none_or(|m| t < m)
        {
            hi = Some(t);
        }
    }
    Ok((lo, hi))
}

/// 按拓扑序重建整条链: 区间内提交替换为新时间，区间后提交保持原时间。
/// 返回 旧 sha -> 新 sha 映射与丢弃的 GPG 签名数。
fn rebuild_chain(
    range: &CommitList,
    descendants: &CommitList,
    times: &[DateTime<Local>],
) -> Result<(HashMap<String, String>, usize)> {
    let mut map = HashMap::new();
    let mut signed_dropped = 0;
    for ((old, _), t) in range.iter().zip(times) {
        let (ts, tz) = raw_date_parts(t);
        let (new, signed) = rebuild_commit(old, Some((&ts, &tz)), &map)?;
        signed_dropped += usize::from(signed);
        eprintln!("{old:.7} -> {t}");
        map.insert(old.clone(), new);
    }
    for (old, _) in descendants {
        let (new, signed) = rebuild_commit(old, None, &map)?;
        signed_dropped += usize::from(signed);
        map.insert(old.clone(), new);
    }
    Ok((map, signed_dropped))
}

/// 其他分支若指向被改写的提交，将停留在旧历史上；仅提示，不阻止
fn warn_diverged_branches(rewritten: &HashMap<String, String>) -> Result<()> {
    let Some(refs) = capture_git(&[
        "for-each-ref",
        "--format=%(refname:short) %(objectname)",
        "refs/heads",
    ])?
    else {
        return Ok(());
    };
    // detached HEAD 时 symbolic-ref 失败返回 None，此时所有分支都算其他分支
    let current = capture_git(&["symbolic-ref", "--short", "HEAD"])?;
    for line in refs.lines() {
        let Some((name, sha)) = line.split_once(' ') else {
            continue
        };
        if current.as_deref() == Some(name) || !rewritten.contains_key(sha) {
            continue;
        }
        eprintln!("警告: 分支 {name} 仍指向旧提交 {sha}，已与改写后的历史分叉");
    }
    Ok(())
}

// --- 配置读取 ---

fn parse_time(s: &str) -> Result<NaiveTime> {
    NaiveTime::parse_from_str(s, "%H:%M")
        .map_err(|_| format!("无效的时间格式: {s}，请使用 HH:MM 格式 (例如 09:00)").into())
}

fn load_time_range() -> Result<(NaiveTime, NaiveTime)> {
    let start_str = read_git_config(CONFIG_KEY_START)?.unwrap_or_else(|| DEFAULT_START.to_string());
    let end_str = read_git_config(CONFIG_KEY_END)?.unwrap_or_else(|| DEFAULT_END.to_string());
    Ok((parse_time(&start_str)?, parse_time(&end_str)?))
}

fn read_git_config(key: &str) -> Result<Option<String>> {
    capture_git(&["config", key])
}

/// 捕获 git 命令的 stdout (trim 后)，失败时返回 None
fn capture_git(args: &[&str]) -> Result<Option<String>> {
    let output = Command::new("git").args(args).output()?;
    if !output.status.success() {
        return Ok(None);
    }
    let s = String::from_utf8_lossy(&output.stdout).trim().to_string();
    Ok((!s.is_empty()).then_some(s))
}

/// 捕获 git 命令的 stdout (trim 后)；与 capture_git 不同，命令失败时直接报错
fn capture_git_strict(args: &[&str]) -> Result<String> {
    let output = Command::new("git").args(args).output()?;
    if !output.status.success() {
        return Err(format!(
            "git {} 失败: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        )
        .into());
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

// --- 提交对象重建 ---

/// `rev-list --reverse --topo-order --parents <range>`，返回拓扑序的 (commit, parents)。
/// 父提交恒排在子提交之前，按序分配递增时间即可保证区间内单调
fn rev_list_with_parents(range: &str) -> Result<CommitList> {
    let out = capture_git_strict(&["rev-list", "--reverse", "--topo-order", "--parents", range])?;
    Ok(out
        .lines()
        .filter_map(|l| {
            let mut parts = l.split_whitespace();
            let commit = parts.next()?;
            Some((commit.to_string(), parts.map(str::to_string).collect()))
        })
        .collect())
}

struct RawCommit {
    tree: Vec<u8>,
    parents: Vec<Vec<u8>>,
    /// author/committer 头的值部分: "Name <email> ts tz"
    author: Vec<u8>,
    committer: Vec<u8>,
    /// 其余头块 (encoding/mergetag 等，含续行)，gpgsig 已剔除
    extra: Vec<Vec<u8>>,
    message: Vec<u8>,
    signed: bool,
}

/// 解析 commit 对象原始字节。头与消息以首个空行分隔；
/// 以空格开头的行是上一个头的续行 (gpgsig/mergetag)
fn parse_raw_commit(raw: &[u8]) -> Option<RawCommit> {
    let sep = raw.windows(2).position(|w| w == b"\n\n")?;
    let (head, rest) = raw.split_at(sep);

    let mut commit = RawCommit {
        tree: Vec::new(),
        parents: Vec::new(),
        author: Vec::new(),
        committer: Vec::new(),
        extra: Vec::new(),
        message: rest[2..].to_vec(),
        signed: false,
    };
    // 收集多行头；gpgsig 在改写后必然失效，标记为丢弃
    let mut pending: Option<(Vec<u8>, bool)> = None;
    for line in head.split(|&b| b == b'\n') {
        if line.first() == Some(&b' ') {
            if let Some((buf, _)) = pending.as_mut() {
                buf.push(b'\n');
                buf.extend_from_slice(line);
            }
            continue;
        }
        if let Some((buf, keep)) = pending.take()
            && keep
        {
            commit.extra.push(buf);
        }
        let sp = line.iter().position(|&b| b == b' ')?;
        let val = &line[sp + 1..];
        match &line[..sp] {
            b"tree" => commit.tree = val.to_vec(),
            b"parent" => commit.parents.push(val.to_vec()),
            b"author" => commit.author = val.to_vec(),
            b"committer" => commit.committer = val.to_vec(),
            b"gpgsig" | b"gpgsig-sha256" => {
                commit.signed = true;
                pending = Some((line.to_vec(), false));
            },
            _ => pending = Some((line.to_vec(), true)),
        }
    }
    if let Some((buf, keep)) = pending.take()
        && keep
    {
        commit.extra.push(buf);
    }
    Some(commit)
}

/// 替换 "Name <email> ts tz" 行末的 ts/tz 两个 token；name/email 可含空格
fn replace_line_date(line: &[u8], ts: &str, tz: &str) -> Option<Vec<u8>> {
    let tz_at = line.iter().rposition(|&b| b == b' ')?;
    let name_end = line[..tz_at].iter().rposition(|&b| b == b' ')?;
    let mut out = line[..name_end].to_vec();
    out.extend_from_slice(format!(" {ts} {tz}").as_bytes());
    Some(out)
}

/// DateTime 转为 git 原始日期格式的两段: (unix 秒, +HHMM 偏移)
fn raw_date_parts(dt: &DateTime<Local>) -> (String, String) {
    (dt.timestamp().to_string(), dt.format("%z").to_string())
}

/// 用 hash-object 重建一个提交：替换日期、映射父提交，其余字节原样保留。
/// 返回 (新 sha, 是否丢弃了 GPG 签名)。parent_map 未命中的父提交保持不变
fn rebuild_commit(
    old: &str,
    new_date: Option<(&str, &str)>,
    parent_map: &HashMap<String, String>,
) -> Result<(String, bool)> {
    let raw = Command::new("git")
        .args(["cat-file", "commit", old])
        .output()?;
    if !raw.status.success() {
        return Err(format!(
            "git cat-file {old} 失败: {}",
            String::from_utf8_lossy(&raw.stderr).trim()
        )
        .into());
    }
    let parsed = parse_raw_commit(&raw.stdout).ok_or_else(|| format!("提交对象格式异常: {old}"))?;

    let (author, committer) = match new_date {
        Some((ts, tz)) => (
            replace_line_date(&parsed.author, ts, tz)
                .ok_or_else(|| format!("author 行格式异常: {old}"))?,
            replace_line_date(&parsed.committer, ts, tz)
                .ok_or_else(|| format!("committer 行格式异常: {old}"))?,
        ),
        None => (parsed.author.clone(), parsed.committer.clone()),
    };

    let mut out = Vec::with_capacity(raw.stdout.len() + 64);
    out.extend_from_slice(b"tree ");
    out.extend_from_slice(&parsed.tree);
    out.push(b'\n');
    for p in &parsed.parents {
        let p = std::str::from_utf8(p).map_err(|_| format!("parent hash 非法: {old}"))?;
        let mapped = parent_map.get(p).map_or(p, String::as_str);
        out.extend_from_slice(b"parent ");
        out.extend_from_slice(mapped.as_bytes());
        out.push(b'\n');
    }
    out.extend_from_slice(b"author ");
    out.extend_from_slice(&author);
    out.push(b'\n');
    out.extend_from_slice(b"committer ");
    out.extend_from_slice(&committer);
    out.push(b'\n');
    for header in &parsed.extra {
        out.extend_from_slice(header);
        out.push(b'\n');
    }
    out.push(b'\n');
    out.extend_from_slice(&parsed.message);
    Ok((hash_object_commit(&out)?, parsed.signed))
}

/// 将构造好的 commit 对象字节写入对象库，返回新 sha
fn hash_object_commit(raw: &[u8]) -> Result<String> {
    let mut child = Command::new("git")
        .args(["hash-object", "-t", "commit", "-w", "--stdin"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    {
        let mut stdin = child.stdin.take().ok_or("打开 stdin 失败")?;
        stdin.write_all(raw)?;
    } // drop 关闭 stdin 使 git 读到 EOF
    let out = child.wait_with_output()?;
    if !out.status.success() {
        return Err(format!(
            "git hash-object 失败: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        )
        .into());
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

// --- 时间生成 ---

/// 获取指定提交 (rev) 的提交时间，返回 None 表示仓库无 commit 或不在仓库中
fn get_commit_time(rev: &str) -> Result<Option<DateTime<Local>>> {
    let output = Command::new("git")
        .args(["show", "-s", "--format=%ct", rev])
        .output();

    let output = match output {
        Ok(o) if o.status.success() => o,
        _ => return Ok(None),
    };

    let s = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if s.is_empty() {
        return Ok(None);
    }
    let ts: i64 = s.parse()?;
    Ok(Local.timestamp_opt(ts, 0).single())
}

/// 获取 HEAD 全部父提交时间的最大值；HEAD 为根提交或读取失败时返回 None。
/// amend 后的新 HEAD 须严格晚于所有父提交，故取最大值而非仅第一父 HEAD^。
fn max_parent_time() -> Result<Option<DateTime<Local>>> {
    // `rev-list --parents` 输出 "<commit> <parent1> <parent2> ..."，
    // 根提交仅输出自身一个 hash
    let Some(out) = capture_git(&["rev-list", "--parents", "-n", "1", "HEAD"])? else {
        return Ok(None);
    };
    let mut max = None;
    for parent in out.split(' ').skip(1) {
        if let Some(t) = get_commit_time(parent)? {
            max = Some(match max {
                Some(m) if m >= t => m,
                _ => t,
            });
        }
    }
    Ok(max)
}

/// 生成随机 commit 时间（实际入口：从 git 读取配置，时间下界由调用方给出）
fn generate_random_commit_time(lower: Option<DateTime<Local>>) -> Result<DateTime<Local>> {
    let (start_time, end_time) = load_time_range()?;
    let now = Local::now();
    match lower {
        Some(lc) => eprintln!("last_commit_time: {lc:?}"),
        None => eprintln!("last_commit_time: <none, 首次提交>"),
    }
    compute_random_time(start_time, end_time, lower, now)
}

/// 计算某一天的时间窗口 [start, end]。跨天窗口 (start >= end) 的 end 落在次日；
/// 日期或时间在本地时区不存在 (DST 间隙) 时返回 None
fn daily_window(
    date: NaiveDate,
    start_time: NaiveTime,
    end_time: NaiveTime,
) -> Option<(DateTime<Local>, DateTime<Local>)> {
    let end_date = if start_time >= end_time {
        date + Duration::days(1)
    } else {
        date
    };
    Some((
        localize_date_time(date, start_time)?,
        localize_date_time(end_date, end_time)?,
    ))
}

/// 核心纯函数：根据时间窗口、上次提交时间、当前时间，计算一个随机 commit 时间。
///
/// 规则：
/// - 新 commit 时间必须严格晚于 `last_commit`（保证单调递增）。
/// - 时间窗口由 (`start_time`, `end_time`) 定义，支持跨天 (`start >= end` 表示次日的 end)。
/// - 优先使用今天（或 `last_commit` 当天，取较晚者）的窗口；窗口耗尽则顺延到次日。
/// - 遇到 DST 间隙自动跳过该日。
fn compute_random_time(
    start_time: NaiveTime,
    end_time: NaiveTime,
    last_commit: Option<DateTime<Local>>,
    now: DateTime<Local>,
) -> Result<DateTime<Local>> {
    let mut target_date = now.date_naive();
    if let Some(lc) = last_commit
        && lc.date_naive() > target_date
    {
        target_date = lc.date_naive();
    }

    for _ in 0..4 {
        let Some((window_start, window_end)) = daily_window(target_date, start_time, end_time)
        else {
            eprintln!("日期 {target_date} 的时间窗口在本地时区不存在 (DST?)，顺延至次日");
            target_date += Duration::days(1);
            continue;
        };

        let effective_start = match last_commit {
            Some(lc) => std::cmp::max(window_start, lc + Duration::seconds(1)),
            None => window_start,
        };

        if effective_start < window_end {
            eprintln!("最终使用的随机区间: {effective_start} - {window_end}");
            let total_seconds = (window_end - effective_start).num_seconds();
            debug_assert!(total_seconds > 0, "区间长度应为正数");
            let mut rng = rand::rng();
            let offset = rng.random_range(0..=total_seconds);
            return Ok(effective_start + Duration::seconds(offset));
        }

        eprintln!("日期 {target_date} 的窗口已耗尽 (last_commit 晚于窗口结束)，顺延至次日");
        target_date += Duration::days(1);
    }

    Err("无法在 4 天内找到有效的提交时间窗口，请检查配置".into())
}

/// 批量采样：在 (lo, hi) 与每日时间窗口的交集内一次性取 n 个严格递增的随机时间。
///
/// - hi 为 None (区间终点即分支 tip) 时无上界，向后逐日扩展窗口直至容量足够，
///   最多扩展到当前日期后第 4 天，与单次 amend 的顺延次数一致。
/// - “一次性”指先在整个并集内均匀取 n 个互不相同的秒偏移再按序映射，
///   而非逐个提交采样——后者每个点都被前一个点约束，会向窗口末尾聚集。
fn compute_random_times(
    start_time: NaiveTime,
    end_time: NaiveTime,
    lo: Option<DateTime<Local>>,
    hi: Option<DateTime<Local>>,
    n: usize,
    now: DateTime<Local>,
) -> Result<Vec<DateTime<Local>>> {
    if n == 0 {
        return Ok(Vec::new());
    }
    // 末提交必须严格早于其后首个未改写提交
    let upper = hi.map(|h| h - Duration::seconds(1));
    // 从 lo 前一天开始枚举，保证跨天窗口 (如 22:00-02:00) 中包含 lo 的尾段被计入；
    // 顺延上限对齐单次 amend: 以 lo 与当前日期中较晚者起算，最多再顺延 4 天
    let first_day = lo.map_or(now.date_naive(), |l| l.date_naive()) - Duration::days(1);
    let day_cap =
        std::cmp::max(now.date_naive(), first_day + Duration::days(1)) + Duration::days(4);

    let mut last_day = upper
        .map_or(now.date_naive(), |u| u.date_naive())
        .max(first_day);
    let intervals: Vec<(DateTime<Local>, i64)>; // (区间起点, 含两端的秒数)
    let total: i64;
    let need = i64::try_from(n).unwrap_or(i64::MAX);
    loop {
        let mut acc = Vec::new();
        let mut day = first_day;
        while day <= last_day {
            if let Some((ws, we)) = daily_window(day, start_time, end_time) {
                let s = lo.map_or(ws, |l| std::cmp::max(ws, l + Duration::seconds(1)));
                let e = upper.map_or(we, |u| std::cmp::min(we, u));
                if s < e {
                    acc.push((s, (e - s).num_seconds() + 1));
                }
            }
            day += Duration::days(1);
        }
        let sum: i64 = acc.iter().map(|&(_, l)| l).sum();
        if sum >= need {
            intervals = acc;
            total = sum;
            break;
        }
        // 有上界时天数固定，无法扩展
        if upper.is_some() || last_day >= day_cap {
            return Err(format!("时间窗口可用秒数不足: 需要 {n} 秒，仅有 {sum} 秒").into());
        }
        last_day += Duration::days(1);
    }

    // 均匀采样 n 个互不相同的偏移；BTreeSet 迭代天然有序
    let mut offsets = BTreeSet::new();
    let mut rng = rand::rng();
    while offsets.len() < n {
        offsets.insert(rng.random_range(0..total));
    }

    // 偏移按各区间长度顺序映射回具体时间
    let mut result = Vec::with_capacity(n);
    let mut iter = intervals.iter().peekable();
    let mut base = 0i64;
    for off in &offsets {
        while let Some(&&(_, len)) = iter.peek()
            && base + len <= *off
        {
            base += len;
            iter.next();
        }
        let &(start, _) = iter.peek().expect("偏移必然落在某个区间内");
        result.push(*start + Duration::seconds(off - base));
    }
    Ok(result)
}

/// 将 (日期, 时间) 转换为本地时区的 DateTime，遇到 DST 间隙或歧义时返回 None
fn localize_date_time(date: NaiveDate, time: NaiveTime) -> Option<DateTime<Local>> {
    let ndt = NaiveDateTime::new(date, time);
    Local.from_local_datetime(&ndt).single()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_dt(y: i32, m: u32, d: u32, h: u32, mi: u32) -> DateTime<Local> {
        let date = NaiveDate::from_ymd_opt(y, m, d).unwrap();
        let time = NaiveTime::from_hms_opt(h, mi, 0).unwrap();
        localize_date_time(date, time).expect("测试日期不应落在 DST 间隙，请使用非转换日的日期")
    }

    fn t(h: u32, m: u32) -> NaiveTime {
        NaiveTime::from_hms_opt(h, m, 0).unwrap()
    }

    #[test]
    fn last_commit_before_window_uses_today_window() {
        // last_commit 昨天 08:00，窗口 09:00-17:00，今天窗口有效
        let start = t(9, 0);
        let end = t(17, 0);
        let last = Some(make_dt(2024, 1, 14, 8, 0));
        let now = make_dt(2024, 1, 15, 12, 0);

        let result = compute_random_time(start, end, last, now).unwrap();
        let ws = make_dt(2024, 1, 15, 9, 0);
        let we = make_dt(2024, 1, 15, 17, 0);
        assert!(
            result >= ws && result <= we,
            "结果 {result} 不在今天窗口 [{ws}, {we}]"
        );
    }

    #[test]
    fn last_commit_inside_window_starts_after_it() {
        let start = t(9, 0);
        let end = t(17, 0);
        let last = Some(make_dt(2024, 1, 15, 14, 0));
        let now = make_dt(2024, 1, 15, 12, 0);

        let result = compute_random_time(start, end, last, now).unwrap();
        let lower = make_dt(2024, 1, 15, 14, 0);
        let upper = make_dt(2024, 1, 15, 17, 0);
        assert!(result > lower, "结果 {result} 必须晚于 last_commit {lower}");
        assert!(result <= upper, "结果 {result} 不得超过窗口结束 {upper}");
    }

    #[test]
    fn last_commit_after_window_rolls_to_next_day() {
        let start = t(9, 0);
        let end = t(17, 0);
        let last = Some(make_dt(2024, 1, 15, 18, 0)); // 晚于今天窗口结束
        let now = make_dt(2024, 1, 15, 12, 0);

        let result = compute_random_time(start, end, last, now).unwrap();
        let ws = make_dt(2024, 1, 16, 9, 0);
        let we = make_dt(2024, 1, 16, 17, 0);
        assert!(
            result >= ws && result <= we,
            "结果 {result} 不在次日窗口 [{ws}, {we}]"
        );
    }

    #[test]
    fn first_commit_has_no_lower_bound() {
        let start = t(9, 0);
        let end = t(17, 0);
        let now = make_dt(2024, 1, 15, 12, 0);

        let result = compute_random_time(start, end, None, now).unwrap();
        let ws = make_dt(2024, 1, 15, 9, 0);
        let we = make_dt(2024, 1, 15, 17, 0);
        assert!(
            result >= ws && result <= we,
            "首次提交结果 {result} 不在窗口 [{ws}, {we}]"
        );
    }

    #[test]
    fn cross_day_window_uses_next_day_end() {
        // 22:00 - 02:00 (跨天)
        let start = t(22, 0);
        let end = t(2, 0);
        let last = Some(make_dt(2024, 1, 15, 21, 0));
        let now = make_dt(2024, 1, 15, 20, 0);

        let result = compute_random_time(start, end, last, now).unwrap();
        let ws = make_dt(2024, 1, 15, 22, 0);
        let we = make_dt(2024, 1, 16, 2, 0);
        assert!(
            result >= ws && result <= we,
            "跨天结果 {result} 不在窗口 [{ws}, {we}]"
        );
    }

    #[test]
    fn cross_day_window_last_commit_inside_window() {
        let start = t(22, 0);
        let end = t(2, 0);
        let last = Some(make_dt(2024, 1, 15, 23, 30)); // 在窗口内
        let now = make_dt(2024, 1, 15, 22, 0);

        let result = compute_random_time(start, end, last, now).unwrap();
        let lower = make_dt(2024, 1, 15, 23, 30);
        let upper = make_dt(2024, 1, 16, 2, 0);
        assert!(result > lower && result <= upper);
    }

    #[test]
    fn future_last_commit_uses_that_date() {
        let start = t(9, 0);
        let end = t(17, 0);
        let last = Some(make_dt(2024, 1, 20, 8, 0)); // 未来
        let now = make_dt(2024, 1, 15, 12, 0);

        let result = compute_random_time(start, end, last, now).unwrap();
        let ws = make_dt(2024, 1, 20, 9, 0);
        let we = make_dt(2024, 1, 20, 17, 0);
        assert!(result >= ws && result <= we);
    }

    #[test]
    fn cross_day_window_exhausted_rolls_forward() {
        // 跨天窗口 22:00-02:00，last_commit 在次日凌晨 03:00 (晚于窗口结束)
        let start = t(22, 0);
        let end = t(2, 0);
        let last = Some(make_dt(2024, 1, 16, 3, 0));
        let now = make_dt(2024, 1, 16, 10, 0);

        let result = compute_random_time(start, end, last, now).unwrap();
        // 应顺延到 1/16 晚上的窗口 (22:00 ~ 次日 02:00)
        let ws = make_dt(2024, 1, 16, 22, 0);
        let we = make_dt(2024, 1, 17, 2, 0);
        assert!(result >= ws && result <= we);
    }

    #[test]
    fn result_is_always_strictly_after_last_commit() {
        // 跑多次，确保随机结果始终严格晚于 last_commit
        let start = t(9, 0);
        let end = t(17, 0);
        let last = make_dt(2024, 1, 15, 16, 59);
        let now = make_dt(2024, 1, 15, 12, 0);

        for _ in 0..50 {
            let result = compute_random_time(start, end, Some(last), now).unwrap();
            assert!(result > last, "结果 {result} 必须严格晚于 last_commit");
        }
    }

    #[test]
    fn batch_times_sorted_and_within_union_of_windows() {
        let start = t(9, 0);
        let end = t(17, 0);
        let lo = make_dt(2024, 1, 10, 8, 0);
        let hi = make_dt(2024, 1, 12, 10, 0);
        let now = make_dt(2024, 1, 15, 12, 0);

        let times = compute_random_times(start, end, Some(lo), Some(hi), 7, now).unwrap();
        assert_eq!(times.len(), 7);
        for w in times.windows(2) {
            assert!(w[0] < w[1], "必须严格递增");
        }
        for x in &times {
            assert!(*x > lo && *x < hi);
            assert!(x.time() >= start && x.time() <= end, "须落在每日窗口内");
        }
    }

    #[test]
    fn batch_times_error_when_seconds_insufficient() {
        let start = t(9, 0);
        let end = t(17, 0);
        // 当日窗口经上下界裁剪后只剩 [16:59:01, 17:00:00] 共 60 秒
        let lo = Some(make_dt(2024, 1, 15, 16, 59));
        let hi = Some(make_dt(2024, 1, 15, 17, 30));
        let now = make_dt(2024, 1, 15, 12, 0);

        assert!(compute_random_times(start, end, lo, hi, 61, now).is_err());
        assert!(compute_random_times(start, end, lo, hi, 60, now).is_ok());
    }

    #[test]
    fn batch_times_roll_forward_when_no_window_left() {
        // 摸鱼窗口 00:00-04:00，下界已是当日 15:00，当日窗口耗尽应顺延次日
        let lo = Some(make_dt(2024, 1, 15, 15, 0));
        let now = make_dt(2024, 1, 15, 15, 30);

        let times = compute_random_times(t(0, 0), t(4, 0), lo, None, 3, now).unwrap();
        let ws = make_dt(2024, 1, 16, 0, 0);
        let we = make_dt(2024, 1, 16, 4, 0);
        for w in times.windows(2) {
            assert!(w[0] < w[1]);
        }
        for x in times {
            assert!(x >= ws && x <= we, "应落在次日窗口 [{ws}, {we}]，实际 {x}");
        }
    }

    #[test]
    fn batch_times_spread_over_days_since_lower_bound() {
        // 下界在 3 天前且无上界：点分布在 (lo, 今天窗口] 的多天窗口并集内
        let lo = make_dt(2024, 1, 10, 10, 0);
        let now = make_dt(2024, 1, 13, 12, 0);

        let times = compute_random_times(t(9, 0), t(17, 0), Some(lo), None, 20, now).unwrap();
        assert_eq!(times.len(), 20);
        for w in times.windows(2) {
            assert!(w[0] < w[1]);
        }
        for x in &times {
            assert!(*x > lo);
            assert!(x.time() >= t(9, 0) && x.time() <= t(17, 0));
            assert!(
                x.date_naive() >= NaiveDate::from_ymd_opt(2024, 1, 10).unwrap()
                    && x.date_naive() <= NaiveDate::from_ymd_opt(2024, 1, 13).unwrap()
            );
        }
    }

    #[test]
    fn parse_raw_commit_drops_signature_keeps_rest() {
        let raw = concat!(
            "tree 1111111111111111111111111111111111111111\n",
            "parent 2222222222222222222222222222222222222222\n",
            "author Foo Bar <foo@bar> 1700000000 +0800\n",
            "committer Foo Bar <foo@bar> 1700000000 +0800\n",
            "encoding UTF-8\n",
            "gpgsig -----BEGIN-----\n",
            " continuation\n",
            "\n",
            "subject\n",
            "\n",
            "body",
        )
        .as_bytes();

        let parsed = parse_raw_commit(raw).unwrap();
        assert_eq!(parsed.tree, b"1111111111111111111111111111111111111111");
        assert_eq!(parsed.parents, vec![
            b"2222222222222222222222222222222222222222".to_vec()
        ]);
        assert!(parsed.signed);
        assert_eq!(parsed.extra, vec![b"encoding UTF-8".to_vec()]);
        assert_eq!(parsed.message, b"subject\n\nbody");

        let replaced = replace_line_date(&parsed.author, "1700000001", "-0500").unwrap();
        assert_eq!(replaced, b"Foo Bar <foo@bar> 1700000001 -0500");
    }
}
