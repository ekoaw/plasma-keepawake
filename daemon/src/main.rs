mod config;
mod engine;
mod providers;

use std::path::PathBuf;
use std::process::ExitCode;

use config::Config;
use engine::RuleEngine;

fn main() -> ExitCode {
    let mut config_path: Option<PathBuf> = None;
    let mut check = false;

    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--check" => check = true,
            "--config" => {
                config_path = args.next().map(PathBuf::from);
            }
            other => {
                eprintln!("unknown argument: {other}");
                return ExitCode::FAILURE;
            }
        }
    }

    let path = config_path.unwrap_or_else(Config::default_path);
    let config = match Config::load(&path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{}: {e}", path.display());
            return ExitCode::FAILURE;
        }
    };

    let rule_engine = RuleEngine::new(&config);
    let statuses = rule_engine.evaluate();

    if check {
        print_check(&statuses);
        return ExitCode::SUCCESS;
    }

    // Milestone 1 ends here: rule evaluation works, but nothing yet acts on
    // it (no D-Bus inhibition, no watching for changes). That's Milestone 3.
    eprintln!("plasma-keepawaked: only `--check` is implemented so far, see PLAN.md");
    print_check(&statuses);
    ExitCode::SUCCESS
}

fn print_check(statuses: &[engine::RuleStatus]) {
    for s in statuses {
        let state = if !s.enabled {
            "disabled".to_string()
        } else {
            match &s.value {
                Ok(true) => "true".to_string(),
                Ok(false) => "false".to_string(),
                Err(e) => format!("error: {e}"),
            }
        };
        println!("{:<28} {state}", s.name);
    }
    println!(
        "\nwould inhibit sleep: {}",
        engine::should_inhibit(statuses)
    );
}
