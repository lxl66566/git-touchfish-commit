use std::process::Command;

use chrono::{DateTime, Duration, Local, NaiveDate, NaiveDateTime, NaiveTime, TimeZone};
use rand::RngExt;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

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
        "amend" => amend_commit_time(&cli[1..])?,
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
    let cross_day = start_time >= end_time;

    let mut target_date = now.date_naive();
    if let Some(lc) = last_commit
        && lc.date_naive() > target_date
    {
        target_date = lc.date_naive();
    }

    for _ in 0..4 {
        let Some(window_start) = localize_date_time(target_date, start_time) else {
            eprintln!("日期 {target_date} {start_time} 在本地时区不存在 (DST?)，顺延至次日");
            target_date += Duration::days(1);
            continue;
        };

        let end_date = if cross_day {
            target_date + Duration::days(1)
        } else {
            target_date
        };
        let Some(window_end) = localize_date_time(end_date, end_time) else {
            eprintln!("日期 {end_date} {end_time} 在本地时区不存在 (DST?)，顺延至次日");
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
}
