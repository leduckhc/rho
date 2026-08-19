//! Wiring for subagents.
//!
//! The `spawn_agent` tool exists in `rho-tools`, and a session cannot reach it until
//! something builds a `SpawnEnv`. That is this module.
//!
//! A tool that no session registers is not shipped. Skills and MCP had the same gap, and
//! it was found the same way: by looking for the tool in the running binary.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use rho_core::{
    AgentNode, AgentRegistry, HookChain, Provider, SessionConfig, SubagentLimits, Tool,
    ToolRegistry,
};
use rho_skills::{AgentConfig, AgentDefinition};
use rho_tools::{ChildToolFactory, SpawnAgentTool, SpawnAgentsTool, SpawnEnv};

/// Build a child's tool registry from the parent's set.
///
/// The parent's tools are held by name, so a child receives the same instances. That is
/// safe because a tool is stateless apart from the `ToolContext` it is handed per call, and
/// the child's context carries the child's own session root.
struct ParentTools {
    /// Every tool the parent advertises, in a stable order.
    tools: Vec<Arc<dyn Tool>>,
}

impl ChildToolFactory for ParentTools {
    fn build(&self, allowed: &[String]) -> ToolRegistry {
        let mut registry = ToolRegistry::new();
        for tool in &self.tools {
            if allowed.iter().any(|name| name == tool.name()) {
                registry.register(Arc::clone(tool));
            }
        }
        registry
    }

    fn parent_tool_names(&self) -> Vec<String> {
        self.tools
            .iter()
            .map(|tool| tool.name().to_string())
            .collect()
    }
}

/// What the caller needs to hold for subagents to work.
pub struct Subagents {
    /// The registry that enforces the process-wide live cap. One per process.
    pub registry: AgentRegistry,
    /// Lines to show the user, for example a withheld project definition.
    pub notices: Vec<String>,
    /// How many definitions loaded. Zero means the tool is not registered.
    pub loaded: usize,
}

/// Discover agent definitions and build the `spawn_agent` tool.
///
/// Returns `None` when no definition loaded. **A tool with nothing to spawn is worse than
/// no tool**, because it costs context in every request and can only ever refuse.
///
/// `trust_project` follows decision D-project-skill-needs-trust, and it matters more here than for a skill. A
/// repository skill is instructions. A repository agent definition is instructions plus a
/// tool list plus a model, and it runs unattended.
pub struct LoadRequest {
    pub session_root: PathBuf,
    pub trust_project: bool,
    pub discover: bool,
    pub parent_config: SessionConfig,
    pub provider: Arc<dyn Provider>,
    pub hooks: Arc<HookChain>,
    /// The parent's tool set, captured **before** `spawn_agent` joins it. So a child can
    /// never receive `spawn_agent` through the intersection, whatever a definition asks for.
    pub parent_tools: Vec<Arc<dyn Tool>>,
    pub limits: SubagentLimits,
}

pub async fn load(request: LoadRequest) -> (Vec<Arc<dyn Tool>>, Subagents) {
    let LoadRequest {
        session_root,
        trust_project,
        discover,
        parent_config,
        provider,
        hooks,
        parent_tools,
        limits,
    } = request;
    let session_root = &session_root;
    let parent_config = &parent_config;
    let mut config = AgentConfig::with_default_user_dirs(session_root);
    config.project_trusted = trust_project;
    config.discover = discover;

    let set = rho_skills::discover_agents(&config).await;
    let mut notices = Vec::new();
    if !set.withheld.is_empty() {
        let names: Vec<&str> = set.withheld.iter().map(|d| d.name.as_str()).collect();
        notices.push(format!(
            "{} project agent definition(s) were found and not loaded: {}. \
             A definition carries a tool list and a model, and it runs unattended, so one \
             from this repository stays off until you trust it. Pass --trust-project.",
            set.withheld.len(),
            names.join(", ")
        ));
    }

    let registry = AgentRegistry::new(limits);
    let loaded = set.loaded.len();
    if loaded == 0 {
        // No definitions, so no tool. See the note on the return type.
        return (
            Vec::new(),
            Subagents {
                registry,
                notices,
                loaded: 0,
            },
        );
    }

    let definitions: HashMap<String, AgentDefinition> = set
        .loaded
        .into_iter()
        .map(|def| (def.name.clone(), def))
        .collect();
    notices.push(format!(
        "{} agent definition(s) available to spawn_agent: {}.",
        definitions.len(),
        {
            let mut names: Vec<&str> = definitions.keys().map(String::as_str).collect();
            names.sort();
            names.join(", ")
        }
    ));

    // A temp directory, not the session root. A transcript under `.rho/` sits inside
    // the user's repository, `.gitignore` does not cover it, and it can be committed
    // by accident. pi writes to a per-user temp root for the same reason.
    let transcript_dir = rho_core::session_transcript_dir(std::process::id());
    let env = SpawnEnv {
        node: root_node(&registry),
        definitions,
        parent_config: parent_config.clone(),
        provider,
        hooks,
        tools: Arc::new(ParentTools {
            tools: parent_tools,
        }),
        transcript_dir,
        // The gate runs a check under the same confinement the parent's bash uses.
        runner: Arc::new(rho_tools::SandboxedRunner::new(parent_config.sandbox)),
        // One ledger per process, so a poisoned task stops being retried.
        retries: Arc::new(rho_core::RetryLedger::new()),
    };
    // Two tools, on purpose. `spawn_agent` is the common single-child case, and
    // `spawn_agents` is a fan-out in one call, because `AgentLoop::dispatch` runs
    // tool calls one at a time. See decision D-fan-out-is-one-tool-call.
    let env = Arc::new(env);
    let tools: Vec<Arc<dyn Tool>> = vec![
        Arc::new(SpawnAgentTool::new(Arc::clone(&env))),
        Arc::new(SpawnAgentsTool::new(Arc::clone(&env))),
        // A running child is addressable, so the model can redirect one instead of
        // cancelling the lot and starting again. See SPEC-steering.
        Arc::new(rho_tools::SteerAgentTool::new(Arc::clone(&env))),
        Arc::new(rho_tools::AgentStatusTool::new(Arc::clone(&env))),
        Arc::new(rho_tools::CancelAgentTool::new(env)),
    ];
    (
        tools,
        Subagents {
            registry,
            notices,
            loaded,
        },
    )
}

/// The root of this session's spawn tree.
fn root_node(registry: &AgentRegistry) -> AgentNode {
    registry.new_tree()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_child_factory_grants_only_the_named_tools() {
        let tools = rho_tools::builtin_tools();
        let factory = ParentTools {
            tools: tools.clone(),
        };
        let names = factory.parent_tool_names();
        assert!(names.contains(&"read".to_string()));

        let registry = factory.build(&["read".to_string(), "grep".to_string()]);
        let granted: Vec<String> = registry.specs().iter().map(|s| s.name.clone()).collect();
        assert_eq!(granted.len(), 2, "only the named tools: {granted:?}");
        assert!(granted.contains(&"read".to_string()));
        assert!(!granted.contains(&"write".to_string()), "{granted:?}");
    }

    #[test]
    fn a_child_factory_ignores_a_name_the_parent_lacks() {
        // The intersection in `rho-core` filters first, so this is defence in depth. A
        // factory that invented a tool would defeat that filter.
        let factory = ParentTools {
            tools: rho_tools::builtin_tools(),
        };
        let registry = factory.build(&["read".to_string(), "not_a_tool".to_string()]);
        let granted: Vec<String> = registry.specs().iter().map(|s| s.name.clone()).collect();
        assert_eq!(granted, vec!["read".to_string()]);
    }
}
