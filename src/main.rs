use chrono::{DateTime, Duration, Local, NaiveTime, TimeZone};
use rand::Rng;
use serde::{Deserialize, Serialize};
use std::process::{Command, Stdio};
use std::str;

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

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli: Vec<String> = std::env::args().skip(1).collect();

    if cli.is_empty() {
        println!("Usage: `git tc set <start> <end>` or `git tc <any other args>`");
        return Ok(());
    }

    if cli[0] == "set" {
        if cli.len() != 3 {
            println!("Usage: `git tc set <start> <end>`, each time should be in HH:MM format");
            return Ok(());
        }
        set_time_range(&cli[1], &cli[2])?;
        return Ok(());
    }

    run_commit(&cli)?;

    Ok(())
}

fn set_time_range(start: &str, end: &str) -> Result<(), Box<dyn std::error::Error>> {
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

fn run_commit(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let cfg: AppConfig = confy::load("git-touchfish-commit", None)?;

    let start_time = NaiveTime::parse_from_str(&cfg.start_time, "%H:%M")?;
    let end_time = NaiveTime::parse_from_str(&cfg.end_time, "%H:%M")?;

    // 生成随机时间
    let now = Local::now();
    let today_date = now.date_naive();

    let start_datetime = today_date.and_time(start_time);
    let end_datetime = today_date.and_time(end_time);

    let mut rng = rand::rng(); // 使用 thread_rng
    let random_duration_seconds =
        rng.random_range(0..=(end_datetime - start_datetime).num_seconds());
    let mut random_datetime = start_datetime + Duration::seconds(random_duration_seconds);

    // 如果随机时间早于当前时间，则将新时间向后推 24 小时
    if random_datetime < now.naive_local() {
        random_datetime += Duration::hours(24);
    }

    let random_datetime_local: DateTime<Local> =
        Local.from_local_datetime(&random_datetime).unwrap();
    let formatted_random_time = random_datetime_local
        .format("%Y-%m-%d %H:%M:%S %z")
        .to_string();

    println!("正在使用时间 {} 执行 git commit...", formatted_random_time);

    // 执行 git commit，并直接设置时间
    let mut commit_command = Command::new("git");
    commit_command.arg("commit");
    commit_command.arg("--date");
    commit_command.arg(&formatted_random_time);
    commit_command.args(args);
    commit_command.env("GIT_COMMITTER_DATE", &formatted_random_time);
    commit_command.env("GIT_AUTHOR_DATE", &formatted_random_time);
    commit_command.stdout(Stdio::inherit());
    commit_command.stderr(Stdio::inherit());

    let status = commit_command.status()?;

    if !status.success() {
        return Err("git commit 失败".into());
    }

    println!("git commit 成功。");

    Ok(())
}
