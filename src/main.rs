use chrono::{DateTime, Duration, Local, NaiveTime, TimeZone};
use rand::Rng;
use serde::{Deserialize, Serialize};
use std::process::Command;

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
fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 跳过程序名称，获取用户传入的参数
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

    // 使用 match 语句处理不同的子命令
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
            // 将 "amend" 命令本身之后的所有参数传递给执行函数
            amend_commit_time(&cli[1..])?;
        }
        // 默认行为是创建一个新的 commit
        _ => {
            run_commit(&cli)?;
        }
    }

    Ok(())
}

// --- 核心功能函数 ---

/// 设置并存储随机时间的起止范围
fn set_time_range(start: &str, end: &str) -> Result<(), Box<dyn std::error::Error>> {
    // 解析时间字符串，如果格式不正确则返回错误
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

    // 使用 confy 库将配置写入文件
    confy::store("git-touchfish-commit", None, cfg)?;
    println!("时间区间已设置为: {} - {}", start, end);
    Ok(())
}

/// 显示当前存储的时间范围
fn show_time_range() -> Result<(), Box<dyn std::error::Error>> {
    let cfg: AppConfig = confy::load("git-touchfish-commit", None)?;
    println!("当前时间区间: {} - {}", cfg.start_time, cfg.end_time);
    Ok(())
}

/// 使用随机时间执行一次新的 git commit
fn run_commit(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    // 生成一个随机时间
    let random_datetime = generate_random_commit_time()?;
    let formatted_time = random_datetime.to_rfc3339();

    println!("正在使用随机时间 {} 执行 git commit...", formatted_time);

    let mut commit_command = Command::new("git");
    commit_command
        .arg("commit")
        .args(args) // 传递用户的所有参数，例如 -m "message"
        .env("GIT_AUTHOR_DATE", &formatted_time)
        .env("GIT_COMMITTER_DATE", &formatted_time);

    // 在子进程中执行命令
    let status = commit_command.status()?;

    if !status.success() {
        return Err("git commit 失败".into());
    }

    println!("git commit 成功。");
    Ok(())
}

/// 使用随机时间修改最后一次 commit
fn amend_commit_time(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    // 生成一个随机时间
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

    // 在子进程中执行命令
    let status = commit_command.status()?;

    if !status.success() {
        return Err("amend 失败".into());
    }

    println!("amend 成功。");
    Ok(())
}

/// 根据存储的配置生成一个随机的 commit 时间
/// 这是被 `run_commit` 和 `amend_commit_time` 复用的核心逻辑
fn generate_random_commit_time() -> Result<DateTime<Local>, Box<dyn std::error::Error>> {
    // 加载配置，如果失败则返回默认值
    let cfg: AppConfig = confy::load("git-touchfish-commit", None)?;

    // 解析配置中的时间
    let start_time = NaiveTime::parse_from_str(&cfg.start_time, "%H:%M")?;
    let end_time = NaiveTime::parse_from_str(&cfg.end_time, "%H:%M")?;

    let now = Local::now();
    let today_date = now.date_naive();

    // 创建今天的起始和结束 datetime 对象
    let start_datetime = today_date.and_time(start_time);
    let end_datetime = today_date.and_time(end_time);

    // 计算总秒数差
    let total_seconds = (end_datetime - start_datetime).num_seconds();
    if total_seconds <= 0 {
        return Err("时间范围无效，结束时间必须晚于开始时间".into());
    }

    // 生成一个随机秒数
    let mut rng = rand::rng();
    let random_offset_seconds = rng.random_range(0..=total_seconds);

    // 计算最终的随机时间
    let mut random_datetime_naive = start_datetime + Duration::seconds(random_offset_seconds);

    // 如果随机生成的时间点在当前时间之前，则将日期推到明天，确保 commit 时间在未来
    if random_datetime_naive < now.naive_local() {
        random_datetime_naive += Duration::days(1);
    }

    // 将 naive 时间转换为带时区的 Local time
    Ok(Local.from_local_datetime(&random_datetime_naive).unwrap())
}
