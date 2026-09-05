mod config;
mod engine;
mod inhibitor;
mod providers;
mod service;
mod state;

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use config::Config;
use engine::{Rule, RuleEngine};
use inhibitor::Inhibitor;
use notify::Watcher;
use state::DaemonState;

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

    if check {
        let mut rule_engine = RuleEngine::new(&config);
        rule_engine.evaluate_all();
        print_check(&rule_engine);
        return ExitCode::SUCCESS;
    }

    if run {
        return run_daemon(path, &config);
    }

    eprintln!("usage: plasma-keepawaked [--config PATH] (--check | --run)");
    ExitCode::FAILURE
}

/// Milestone 4: the actual persistent daemon. Serves
/// `org.plasmakeepawake.Daemon1` on the session bus, hot-reloads the config
/// on file change, and holds/releases the logind sleep inhibitor on a
/// fixed poll interval (still polling, not fully event-driven — see
/// PLAN.md on why that's deferred rather than built speculatively).
fn run_daemon(config_path: PathBuf, config: &Config) -> ExitCode {
    let state = Arc::new(Mutex::new(DaemonState::new(config_path.clone(), config)));

    let iface = service::DaemonIface {
        state: state.clone(),
    };
    let _connection = match zbus::blocking::connection::Builder::session()
        .and_then(|b| b.name(service::BUS_NAME))
        .and_then(|b| b.serve_at(service::OBJECT_PATH, iface))
        .and_then(|b| b.build())
    {
        Ok(c) => c,
        Err(e) => {
            eprintln!("failed to start D-Bus service ({}): {e}", service::BUS_NAME);
            return ExitCode::FAILURE;
        }
    };

    let _watcher = watch_config(config_path, state.clone());

    println!(
        "plasma-keepawaked: running, serving {} (Ctrl+C to stop)",
        service::BUS_NAME
    );

    let mut inhibitor = Inhibitor::new();
    loop {
        let (should_inhibit, reason) = {
            let mut st = state.lock().unwrap();
            st.rule_engine.evaluate_all();
            for rule in st.rule_engine.rules() {
                if let Err(e) = &rule.value {
                    eprintln!("rule {:?}: {e}", rule.name);
                }
            }
            let reason = st.rule_engine.active_rule_names().join(", ");
            let should_inhibit = st.rule_engine.should_inhibit();
            st.inhibiting = should_inhibit;
            st.reason = reason.clone();
            (should_inhibit, reason)
        };

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

/// Watches the config file's parent directory (not the file itself — an
/// editor's save-by-rename would otherwise orphan a direct file watch) and
/// reloads on any event touching it.
fn watch_config(
    path: PathBuf,
    state: Arc<Mutex<DaemonState>>,
) -> Option<notify::RecommendedWatcher> {
    let dir = path.parent()?.to_path_buf();
    let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        let Ok(event) = res else { return };
        if event.paths.iter().any(|p| p == &path) {
            state.lock().unwrap().reload();
        }
    })
    .ok()?;
    watcher
        .watch(&dir, notify::RecursiveMode::NonRecursive)
        .ok()?;
    Some(watcher)
}

fn print_check(rule_engine: &RuleEngine) {
    for rule in rule_engine.rules() {
        print_rule(rule);
    }
    println!("\nwould inhibit sleep: {}", rule_engine.should_inhibit());
}

fn print_rule(rule: &Rule) {
    let state = if !rule.enabled {
        "disabled".to_string()
    } else {
        match &rule.value {
            Ok(true) => "true".to_string(),
            Ok(false) => "false".to_string(),
            Err(e) => format!("error: {e}"),
        }
    };
    println!("{:<28} {state}", rule.name);
}
