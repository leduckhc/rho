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
    let mut notices = notices_for(&set);

    let registry = AgentRegistry::new(limits);
    let loaded = set.loaded.len();
    if loaded == 0 {
        // No definitions, so no tool. See the note on the return type. The notices still
        // go back, because this is the case that removes the tool from the session.
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
    notices.push(available_notice(&definitions));

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

/// The most rejection lines to print. The rest are counted.
///
/// A directory of broken files must not push the real output off the screen.
const MAX_REJECTION_LINES: usize = 5;

/// Every line a discovery pass owes the user, in one place.
///
/// One place, because a second builder is how a report gets lost. The set holds three
/// lists, and each one has a line here.
fn notices_for(set: &rho_skills::AgentSet) -> Vec<String> {
    let mut notices = Vec::new();

    // A rejected file first, because it is the one the user must repair. It used to be
    // dropped in silence, and a whole directory of rejects removed `spawn_agent`.
    for rejected in set.rejected.iter().take(MAX_REJECTION_LINES) {
        notices.push(rejected.notice());
    }
    let hidden = set.rejected.len().saturating_sub(MAX_REJECTION_LINES);
    if hidden > 0 {
        notices.push(format!(
            "{hidden} more agent definition file(s) are not listed here. Repair the files \
             above, then start rho again."
        ));
    }

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

    notices
}

/// The line that lists what the model may spawn.
fn available_notice(definitions: &HashMap<String, AgentDefinition>) -> String {
    let mut names: Vec<&str> = definitions.keys().map(String::as_str).collect();
    names.sort();
    format!(
        "{} agent definition(s) available to spawn_agent: {}.",
        definitions.len(),
        names.join(", ")
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use rho_skills::AgentConfig;
    use std::path::Path;

    /// Write one agent definition file, and return its path.
    fn write_agent(dir: &Path, file: &str, body: &str) -> std::path::PathBuf {
        std::fs::create_dir_all(dir).unwrap();
        let path = dir.join(file);
        std::fs::write(&path, body).unwrap();
        path
    }

    /// Discover only the definitions in one directory. It never reads a real home.
    async fn discover_in(dir: &Path) -> rho_skills::AgentSet {
        rho_skills::discover_agents(&AgentConfig {
            user_dirs: vec![dir.to_path_buf()],
            session_root: None,
            project_trusted: false,
            discover: true,
        })
        .await
    }

    const GOOD: &str = "---\nname: scout\ndescription: Recon.\ntools: read\n---\nbody\n";
    const BROKEN: &str = "---\nname: broken\ndescription: Recon.\ntools: 5\n---\nbody\n";

    #[tokio::test]
    async fn a_rejected_definition_reaches_the_user_as_a_notice() {
        let dir = tempfile::tempdir().unwrap();
        write_agent(dir.path(), "scout.md", GOOD);
        let broken = write_agent(dir.path(), "broken.md", BROKEN);

        let set = discover_in(dir.path()).await;
        let notices = notices_for(&set);
        let joined = notices.join("\n");
        assert!(
            joined.contains(&broken.display().to_string()),
            "the notice names the file: {joined}"
        );
        assert!(
            joined.contains("did not load"),
            "the notice says the file did not load: {joined}"
        );
        assert!(
            joined.contains("tools: [read, list]"),
            "the notice teaches the repair: {joined}"
        );
    }

    #[tokio::test]
    async fn a_rejection_is_reported_even_when_no_definition_loaded() {
        // The silence this change repairs. Every file is broken, so no definition
        // loads, so rho registers no spawn_agent at all. The user must be told why.
        let dir = tempfile::tempdir().unwrap();
        write_agent(dir.path(), "broken.md", BROKEN);

        let set = discover_in(dir.path()).await;
        assert!(set.loaded.is_empty(), "nothing loaded, so no tool");
        let joined = notices_for(&set).join("\n");
        assert!(
            joined.contains("did not load"),
            "a session with no tool must still say why: {joined}"
        );
    }

    #[tokio::test]
    async fn a_flood_of_rejections_is_capped_and_counted() {
        // A directory of broken files must not push the real output off the screen.
        let dir = tempfile::tempdir().unwrap();
        for index in 0..6 {
            write_agent(dir.path(), &format!("broken-{index}.md"), BROKEN);
        }

        let set = discover_in(dir.path()).await;
        assert_eq!(set.rejected.len(), 6);
        let notices = notices_for(&set);
        let lines = notices
            .iter()
            .filter(|line| line.contains("did not load"))
            .count();
        assert_eq!(lines, MAX_REJECTION_LINES, "at most five files are named");
        assert!(
            notices.iter().any(|line| line.contains("1 more")),
            "the rest are counted: {notices:?}"
        );
    }

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
