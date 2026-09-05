use rhai::{Engine, AST};

use crate::config::Config;
use crate::providers;

pub struct Rule {
    pub name: String,
    pub enabled: bool,
    ast: Result<AST, String>,
    /// Last value from `evaluate_all`. Kept even for disabled rules so a
    /// future UI can show "this would be true but you disabled it."
    pub value: Result<bool, String>,
}

/// Compiled rules plus their live `enabled`/`value` state, sharable behind
/// a `Mutex` (rhai's `sync` feature makes `Engine`/`AST` `Send + Sync`) so
/// both the poll loop and the D-Bus service can read/mutate it.
pub struct RuleEngine {
    engine: Engine,
    rules: Vec<Rule>,
}

impl RuleEngine {
    pub fn new(config: &Config) -> Self {
        let mut engine = Engine::new();
        providers::register_all(&mut engine);

        let rules = config
            .rules
            .iter()
            .map(|rule| Rule {
                name: rule.name.clone(),
                enabled: rule.enabled,
                ast: engine
                    .compile_expression(&rule.expr)
                    .map_err(|e| e.to_string()),
                value: Err("not yet evaluated".to_string()),
            })
            .collect();

        RuleEngine { engine, rules }
    }

    /// Re-evaluates every rule's `expr` against the providers' current
    /// state, regardless of `enabled`. Cheap (no I/O; providers do their
    /// own D-Bus/proc/fs queries synchronously inside the Rhai call).
    pub fn evaluate_all(&mut self) {
        for rule in &mut self.rules {
            rule.value = match &rule.ast {
                Ok(ast) => self
                    .engine
                    .eval_ast::<bool>(ast)
                    .map_err(|e| e.to_string()),
                Err(e) => Err(e.clone()),
            };
        }
    }

    pub fn rules(&self) -> &[Rule] {
        &self.rules
    }

    /// True while any *enabled* rule's last-evaluated value is `Ok(true)`.
    pub fn should_inhibit(&self) -> bool {
        self.rules
            .iter()
            .any(|r| r.enabled && matches!(r.value, Ok(true)))
    }

    pub fn active_rule_names(&self) -> Vec<&str> {
        self.rules
            .iter()
            .filter(|r| r.enabled && matches!(r.value, Ok(true)))
            .map(|r| r.name.as_str())
            .collect()
    }

    /// Returns `true` if a rule named `name` was found and updated.
    pub fn set_enabled(&mut self, name: &str, enabled: bool) -> bool {
        match self.rules.iter_mut().find(|r| r.name == name) {
            Some(r) => {
                r.enabled = enabled;
                true
            }
            None => false,
        }
    }
}
