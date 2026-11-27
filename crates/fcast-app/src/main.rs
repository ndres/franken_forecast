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
    let cmd = args.get(1).map(String::as_str);

    let result = match cmd {
        None | Some("help" | "--help" | "-h") => Ok(USAGE.to_owned()),
        Some("init") => commands::run_init_cli(&args[2..]),
        Some("verify-receipts") => commands::run_verify_receipts_cli(&args[2..]),
        Some("gemini") => commands::run_gemini_cli(&args[2..]),
        Some("demo") => commands::run_demo_cli(&args[2..]),
        Some("forecast") => commands::run_forecast_cli(&args[2..]),
        Some(other) => Err(format!("unknown command: {other}\n\n{USAGE}").into()),
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
