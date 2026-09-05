mod config;
mod engine;
mod inhibitor;
mod providers;

use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use config::Config;
use engine::RuleEngine;
use inhibitor::Inhibitor;

const POLL_INTERVAL: Duration = Duration::from_secs(2);

fn main() -> ExitCode {
    let mut config_path: Option<PathBuf> = None;
    let mut check = false;
    let mut run = false;

    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--check" => check = true,
            "--run" => run = true,
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

    if check {
        print_check(&rule_engine.evaluate());
        return ExitCode::SUCCESS;
    }

    if run {
        return run_loop(&rule_engine);
    }

    eprintln!("usage: plasma-keepawaked [--config PATH] (--check | --run)");
    ExitCode::FAILURE
}

/// Milestone 3: evaluate rules on a fixed interval and hold/release a real
/// logind sleep inhibitor accordingly. No D-Bus service or hot-reload yet
/// (Milestone 4) — this is deliberately just enough to verify the
/// inhibition mechanics against real desktop behavior.
fn run_loop(rule_engine: &RuleEngine) -> ExitCode {
    let mut inhibitor = Inhibitor::new();
    println!("plasma-keepawaked: running (Ctrl+C to stop)");

    loop {
        let statuses = rule_engine.evaluate();
        let active: Vec<&str> = statuses
            .iter()
            .filter(|s| s.enabled && matches!(s.value, Ok(true)))
            .map(|s| s.name.as_str())
            .collect();
        for s in &statuses {
            if let Err(e) = &s.value {
                eprintln!("rule {:?}: {e}", s.name);
            }
        }

        let should_inhibit = !active.is_empty();
        let reason = active.join(", ");
        if inhibitor.reconcile(should_inhibit, &reason) {
            if should_inhibit {
                println!("inhibiting sleep: {reason}");
            } else {
                println!("no active rules, released sleep inhibition");
            }
        }

        std::thread::sleep(POLL_INTERVAL);
    }
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
