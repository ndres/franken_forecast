//! `fcast` CLI & `fcast-verify` offline receipt auditor (`M0`, `M1.5`, `GATE-24`, `§0.2` Rule 7).

mod commands;

use std::env;
use std::process::ExitCode;

const USAGE: &str = "\
FrankenForecast (fcast) CLI
Usage:
  fcast init [--data-root <dir>]
  fcast demo [--data-root <dir>] [--live-gemini]
  fcast forecast <question_id> --title <t> --as-of <utc> [--model <m>] [--dry-run] [--data-root <dir>]
  fcast verify-receipts [--write-dir <dir>]
  fcast gemini whoami
  fcast gemini models
  fcast gemini generate <prompt> [--model <m>] [--system <s>] [--previous-interaction-id <id>] [--dry-run]
";

fn main() -> ExitCode {
    let args: Vec<String> = env::args().collect();
    let cmd = args.get(1).map(String::as_str).unwrap_or("help");

    let result = match cmd {
        "init" => commands::run_init_cli(&args[2..]),
        "verify-receipts" => commands::run_verify_receipts_cli(&args[2..]),
        "gemini" => commands::run_gemini_cli(&args[2..]),
        "demo" => commands::run_demo_cli(&args[2..]),
        "forecast" => commands::run_forecast_cli(&args[2..]),
        _ => Ok(USAGE.to_owned()),
    };

    match result {
        Ok(out) => {
            println!("{out}");
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("fcast error: {err}");
            ExitCode::FAILURE
        }
    }
}
