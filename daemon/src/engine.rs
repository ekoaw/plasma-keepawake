use rhai::{Engine, AST};

use crate::config::Config;
use crate::providers;

pub struct CompiledRule {
    pub name: String,
    pub enabled: bool,
    ast: Result<AST, String>,
}

pub struct RuleStatus {
    pub name: String,
    pub enabled: bool,
    pub value: Result<bool, String>,
}

pub struct RuleEngine {
    engine: Engine,
    rules: Vec<CompiledRule>,
}

impl RuleEngine {
    pub fn new(config: &Config) -> Self {
        let mut engine = Engine::new();
        providers::register_all(&mut engine);

        let rules = config
            .rules
            .iter()
            .map(|rule| {
                let ast = engine
                    .compile_expression(&rule.expr)
                    .map_err(|e| e.to_string());
                CompiledRule {
                    name: rule.name.clone(),
                    enabled: rule.enabled,
                    ast,
                }
            })
            .collect();

        RuleEngine { engine, rules }
    }

    /// Evaluates every rule against the providers' current state. Cheap
    /// enough to call on every provider-cache update once real providers
    /// exist (Milestone 2) — no I/O happens here, only Rhai evaluation.
    pub fn evaluate(&self) -> Vec<RuleStatus> {
        self.rules
            .iter()
            .map(|rule| {
                let value = match &rule.ast {
                    Ok(ast) => self
                        .engine
                        .eval_ast::<bool>(ast)
                        .map_err(|e| e.to_string()),
                    Err(e) => Err(e.clone()),
                };
                RuleStatus {
                    name: rule.name.clone(),
                    enabled: rule.enabled,
                    value,
                }
            })
            .collect()
    }
}

/// True while any *enabled* rule evaluates to `true`. A rule with a compile
/// or eval error is treated as false rather than crashing the daemon.
pub fn should_inhibit(statuses: &[RuleStatus]) -> bool {
    statuses
        .iter()
        .any(|s| s.enabled && matches!(s.value, Ok(true)))
}
