use chrono::{DateTime, Duration, Local, NaiveTime, TimeZone};
use rand::Rng;
use serde::{Deserialize, Serialize};
use std::process::Command;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

// --- 配置结构体 ---
#[derive(Debug, Serialize, Deserialize)]
struct AppConfig {
    start_time: String,
    end_time: String,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            start_time: "00:00".to_string(),
            end_time: "02:00".to_string(),
        }
    }
}

// --- 主函数：命令行入口 ---
fn main() -> Result<()> {
    let cli: Vec<String> = std::env::args().skip(1).collect();

    if cli.is_empty() {
        println!(
            r#"Usage:
git-tc set <start> <end>
git-tc show
git-tc amend
git-tc ...
"#
        );
        return Ok(());
    }

    match cli[0].as_str() {
        "set" => {
            if cli.len() != 3 {
                println!("用法: `git-tc set <start> <end>`, 时间应为 HH:MM 格式");
                return Ok(());
            }
            set_time_range(&cli[1], &cli[2])?;
        }
        "show" => {
            show_time_range()?;
        }
        "amend" => {
            amend_commit_time(&cli[1..])?;
        }
        _ => {
            run_commit(&cli)?;
        }
    }

    Ok(())
}

// --- 核心功能函数 ---

fn set_time_range(start: &str, end: &str) -> Result<()> {
    let start_time = NaiveTime::parse_from_str(start, "%H:%M").map_err(|_| {
        format!(
            "无效的开始时间格式: {}. 请使用 HH:MM 格式 (例如 09:00)",
            start
        )
    })?;
    let end_time = NaiveTime::parse_from_str(end, "%H:%M").map_err(|_| {
        format!(
            "无效的结束时间格式: {}. 请使用 HH:MM 格式 (例如 17:00)",
            end
        )
    })?;

    if start_time >= end_time {
        return Err("开始时间必须早于结束时间".into());
    }

    let cfg = AppConfig {
        start_time: start.to_string(),
        end_time: end.to_string(),
    };

    confy::store("git-touchfish-commit", None, cfg)?;
    println!("时间区间已设置为: {} - {}", start, end);
    Ok(())
}

fn show_time_range() -> Result<()> {
    let cfg: AppConfig = confy::load("git-touchfish-commit", None)?;
    println!("当前时间区间: {} - {}", cfg.start_time, cfg.end_time);
    Ok(())
}

fn run_commit(args: &[String]) -> Result<()> {
    let random_datetime = generate_random_commit_time()?;
    let formatted_time = random_datetime.to_rfc3339();

    println!("正在使用随机时间 {} 执行 git commit...", formatted_time);

    let mut commit_command = Command::new("git");
    commit_command
        .arg("commit")
        .args(args)
        .env("GIT_AUTHOR_DATE", &formatted_time)
        .env("GIT_COMMITTER_DATE", &formatted_time);

    let status = commit_command.status()?;

    if !status.success() {
        return Err("git commit 失败".into());
    }

    println!("git commit 成功。");
    Ok(())
}

fn amend_commit_time(args: &[String]) -> Result<()> {
    let random_datetime = generate_random_commit_time()?;
    let formatted_time = random_datetime.to_rfc3339();

    println!("正在使用随机时间 {} 修改最后一次 commit...", formatted_time);

    let mut commit_command = Command::new("git");
    commit_command
        .arg("commit")
        .arg("--amend")
        .arg("--no-edit")
        .arg("--reset-author")
        .args(args)
        .env("GIT_AUTHOR_DATE", &formatted_time)
        .env("GIT_COMMITTER_DATE", &formatted_time);

    let status = commit_command.status()?;

    if !status.success() {
        return Err("amend 失败".into());
    }

    println!("amend 成功。");
    Ok(())
}

/// 获取当前仓库最后一次 commit 的时间
fn get_last_commit_time() -> Result<DateTime<Local>> {
    // 使用 git log -1 --format=%ct 获取最后一次提交的 Unix 时间戳
    let output = Command::new("git")
        .args(["log", "-1", "--format=%ct"])
        .output();

    // 如果执行失败（例如不在 git 仓库中，或者没有 commit），默认返回一个很久以前的时间
    // 这样逻辑就会回退到使用 "今天"
    let output = match output {
        Ok(o) if o.status.success() => o,
        _ => return Ok(Local.timestamp_opt(0, 0).unwrap()), // 1970-01-01
    };

    let timestamp_str = String::from_utf8(output.stdout)?.trim().to_string();
    if timestamp_str.is_empty() {
        return Ok(Local.timestamp_opt(0, 0).unwrap());
    }

    let timestamp: i64 = timestamp_str.parse()?;
    // 将时间戳转换为本地时间
    Ok(Local.timestamp_opt(timestamp, 0).unwrap())
}

/// 生成随机时间，保证晚于最后一次 commit
fn generate_random_commit_time() -> Result<DateTime<Local>> {
    let cfg: AppConfig = confy::load("git-touchfish-commit", None)?;

    let start_time = NaiveTime::parse_from_str(&cfg.start_time, "%H:%M")?;
    let end_time = NaiveTime::parse_from_str(&cfg.end_time, "%H:%M")?;

    // 1. 获取最后一次 commit 的时间
    let last_commit_time = get_last_commit_time()?;

    // 2. 基础日期默认为“今天”
    let now = Local::now();
    let mut base_date = now.date_naive();

    // 如果最后一次提交的时间比今天还晚（比如之前已经做过未来的提交），
    // 那么基础日期至少要从那一天开始，否则生成的“今天”肯定会早于“最后提交”
    if last_commit_time.date_naive() > base_date {
        base_date = last_commit_time.date_naive();
    }

    // 3. 在基础日期上构建随机时间
    let start_datetime = base_date.and_time(start_time);
    let end_datetime = base_date.and_time(end_time);

    let total_seconds = (end_datetime - start_datetime).num_seconds();
    if total_seconds <= 0 {
        return Err("时间范围无效，结束时间必须晚于开始时间".into());
    }

    let mut rng = rand::rng();
    let random_offset_seconds = rng.random_range(0..=total_seconds);

    // 初始生成的随机时间
    let mut random_datetime_naive = start_datetime + Duration::seconds(random_offset_seconds);
    let mut final_datetime = Local.from_local_datetime(&random_datetime_naive).unwrap();

    // 4. 核心逻辑：如果生成的随机时间 <= 最后一次提交时间，则顺延一天
    // 这种情况通常发生在：
    // a. 今天已经提交过了，且最后一次提交时间晚于刚才随机出的时间。
    // b. 设定的时间区间（如 09:00-10:00）整体早于最后一次提交时间（如 11:00）。
    if final_datetime <= last_commit_time {
        println!(
            "生成的随机时间 ({}) 早于最后一次提交 ({})，自动顺延一天...",
            final_datetime.format("%Y-%m-%d %H:%M:%S"),
            last_commit_time.format("%Y-%m-%d %H:%M:%S")
        );

        random_datetime_naive += Duration::days(1);
        final_datetime = Local.from_local_datetime(&random_datetime_naive).unwrap();
    }

    Ok(final_datetime)
}
