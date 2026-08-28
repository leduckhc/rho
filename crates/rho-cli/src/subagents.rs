//! Wiring for subagents.
//!
//! The `spawn_agent` tool exists in `rho-tools`, and a session cannot reach it until
//! something builds a `SpawnEnv`. That is this module.
//!
//! A tool that no session registers is not shipped. Skills and MCP had the same gap, and
//! it was found the same way: by looking for the tool in the running binary.

use std::collections::HashMap;
use std::sync::Arc;

use crate::extensions::summarise_names;
use rho_core::{
    AgentNode, AgentRegistry, HookChain, Provider, SessionConfig, SubagentLimits, Tool,
    ToolRegistry,
};
use rho_skills::{AgentConfig, AgentDefinition, MAX_LINES_PER_KIND};
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
    /// Where to look for a definition, and what to trust.
    ///
    /// The caller builds this, so `load` reads no environment variable. A test can then
    /// point discovery at a temporary directory, and the real `~/.rho/agents` of the
    /// machine running the test cannot change the result.
    pub agents: AgentConfig,
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
        agents,
        parent_config,
        provider,
        hooks,
        parent_tools,
        limits,
    } = request;
    let parent_config = &parent_config;

    let set = rho_skills::discover_agents(&agents).await;
    let mut notices = notices_for(&set);

    let registry = AgentRegistry::new(limits);
    let loaded = set.loaded.len();
    if loaded == 0 {
        // No definitions, so no tool. See the note on the return type. **The notices go
        // back on this path too**, because this is the case that removes the tool from
        // the session. One `Subagents` is built, at the end, so this return cannot drop
        // them by accident.
        return (
            Vec::new(),
            Subagents {
                registry,
                notices,
                loaded,
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

/// Every line a discovery pass owes the user, in one place.
///
/// One place, because a second builder is how a report gets lost. The set holds three
/// lists, and each one has a line here.
fn notices_for(set: &rho_skills::AgentSet) -> Vec<String> {
    let mut notices = Vec::new();

    // A rejected file first, because it is the one the user must repair. It used to be
    // dropped in silence, and a whole directory of rejects removed `spawn_agent`.
    for rejected in set.rejected.iter().take(MAX_LINES_PER_KIND) {
        notices.push(rejected.notice());
    }
    let hidden = set.rejected.len().saturating_sub(MAX_LINES_PER_KIND);
    if hidden > 0 {
        notices.push(format!(
            "{hidden} more agent definition file(s) are not listed here. Repair the files \
             above, then start rho again."
        ));
    }

    if !set.withheld.is_empty() {
        // Every name is bounded and summarised. A repository chooses its own names, and
        // fifty files with a 16 KiB name each printed 800 KB on one line.
        let names: Vec<String> = set.withheld.iter().map(|d| d.safe_name()).collect();
        notices.push(format!(
            "{} project agent definition(s) were found and not loaded: {}. \
             A definition carries a tool list and a model, and it runs unattended, so one \
             from this repository stays off until you trust it. Pass --trust-project.",
            set.withheld.len(),
            summarise_names(&names)
        ));
    }

    // A definition that loaded may still hold a warning, for example a dropped tool
    // keyword or a bad sandbox value. Nothing printed one, so a definition narrowed a
    // child in silence. A skill warning has always printed.
    //
    // Only a loaded definition reports. A withheld one changes nothing in this session,
    // and its warning would carry prose from a repository the user has not trusted.
    //
    // The budget counts **lines**, not definitions. Counting definitions let five of
    // them print four lines each, which is twenty lines under a cap of five. Each
    // definition bounds its own lines as well, so neither one file nor many can flood.
    let mut budget = MAX_LINES_PER_KIND;
    let mut unlisted = 0;
    for def in &set.loaded {
        for line in def.notices() {
            if budget == 0 {
                unlisted += 1;
                continue;
            }
            notices.push(line);
            budget -= 1;
        }
    }
    if unlisted > 0 {
        notices.push(format!(
            "{unlisted} more warning line(s) about an agent definition are not listed here."
        ));
    }

    notices
}

/// The line that lists what the model may spawn.
fn available_notice(definitions: &HashMap<String, AgentDefinition>) -> String {
    let mut names: Vec<String> = definitions
        .values()
        .map(AgentDefinition::safe_name)
        .collect();
    names.sort();
    format!(
        "{} agent definition(s) available to spawn_agent: {}.",
        definitions.len(),
        summarise_names(&names)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use rho_skills::AgentConfig;
    use std::path::Path;

    /// A provider that refuses every request. `load` never calls it, because these
    /// tests spawn no child. It exists so the production `load` can be tested at all.
    struct NeverProvider;

    #[async_trait::async_trait]
    impl rho_core::Provider for NeverProvider {
        fn id(&self) -> &str {
            "never"
        }

        async fn stream(
            &self,
            _request: rho_core::CompletionRequest,
            _cancel: rho_core::CancelToken,
        ) -> Result<rho_core::ProviderStream, rho_core::ProviderError> {
            unreachable!("no test here spawns a child")
        }
    }

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
        // The invariant is "the notice carries the reason's own repair", not a literal
        // fragment of today's wording. A reworded repair must not fail this test, and a
        // dropped repair must.
        let repair = set.rejected[0].reason.repair();
        assert!(
            joined.contains(repair),
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
        // The invariant, not the example: however many files break, the report names at
        // most the cap and counts the remainder exactly.
        assert_eq!(
            lines,
            set.rejected.len().min(MAX_LINES_PER_KIND),
            "the report names at most the cap: {notices:?}"
        );
        let hidden = set.rejected.len() - lines;
        assert!(
            notices
                .iter()
                .any(|line| line.contains(&format!("{hidden} more"))),
            "the remainder is counted exactly: {notices:?}"
        );
    }

    #[tokio::test]
    async fn a_flood_of_warnings_is_capped_and_counted() {
        // The cap protected the rejection lines only. A live run printed 104 lines and
        // 1.6 MB, because every definition that loaded added an uncapped warning.
        //
        // Each file here raises **three** warnings, so a budget that counts definitions
        // instead of lines would print fifteen lines under a cap of five. A first version
        // of this cap did exactly that.
        let dir = tempfile::tempdir().unwrap();
        for index in 0..9 {
            write_agent(
                dir.path(),
                &format!("warner-{index}.md"),
                &format!(
                    "---\nname: Warner {index}\ndescription: Recon.\ntools: all, read\n\
                     sandbox: nonsense\n---\nbody\n"
                ),
            );
        }

        let set = discover_in(dir.path()).await;
        assert_eq!(set.loaded.len(), 9, "every file loads");
        let total: usize = set.loaded.iter().map(|d| d.warnings.len()).sum();
        assert!(total >= 27, "each file raises three warnings: {total}");

        let notices = notices_for(&set);
        let warning_lines = notices
            .iter()
            .filter(|line| line.starts_with("agent definition "))
            .count();
        assert_eq!(
            warning_lines, MAX_LINES_PER_KIND,
            "the budget counts lines, not definitions: {notices:?}"
        );
        let unlisted: usize = notices
            .iter()
            .rev()
            .find_map(|line| line.split(' ').next()?.parse::<usize>().ok())
            .expect("a counted line");
        assert!(unlisted > 0, "the rest are counted: {notices:?}");
    }

    #[tokio::test]
    async fn a_hostile_name_cannot_flood_a_notice_line() {
        // A repository chooses its own names, and a name only warns above 64 characters.
        // Fifty files with a 16 KiB name each printed 800 KB on one withheld line.
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let long = "x".repeat(16_000);
        for index in 0..8 {
            write_agent(
                &root.join(".rho").join("agents"),
                &format!("a{index}.md"),
                &format!("---\nname: {long}\ndescription: Recon.\n---\nbody\n"),
            );
        }
        let set = rho_skills::discover_agents(&rho_skills::AgentConfig {
            user_dirs: Vec::new(),
            session_root: Some(root.to_path_buf()),
            project_trusted: false,
            discover: true,
        })
        .await;
        assert_eq!(set.withheld.len(), 8);

        for line in notices_for(&set) {
            assert!(
                line.chars().count() < 600,
                "no start-up line may run away: {} characters",
                line.chars().count()
            );
        }
    }

    #[tokio::test]
    async fn load_reports_a_rejection_when_no_definition_loaded() {
        // The production path, not a helper. `load` owns the early return for zero
        // definitions, and that return is the one that removes the tool from the
        // session. A test on `notices_for` alone would still pass if the return
        // dropped its notices.
        let dir = tempfile::tempdir().unwrap();
        let user = dir.path().join("agents");
        write_agent(&user, "broken.md", BROKEN);

        let (tools, subagents) = load(LoadRequest {
            agents: rho_skills::AgentConfig {
                user_dirs: vec![user],
                session_root: None,
                project_trusted: false,
                discover: true,
            },
            parent_config: SessionConfig::new(
                "test-model",
                dir.path().to_path_buf(),
                Arc::new(rho_core::AllowAllPolicy),
            ),
            provider: Arc::new(NeverProvider),
            hooks: Arc::new(HookChain::new()),
            parent_tools: Vec::new(),
            limits: SubagentLimits::new(),
        })
        .await;

        assert!(tools.is_empty(), "no definition loaded, so no tool");
        assert_eq!(subagents.loaded, 0);
        let joined = subagents.notices.join("\n");
        assert!(
            joined.contains("did not load"),
            "the session lost five tools, so it must say why: {joined}"
        );
    }

    #[tokio::test]
    async fn load_lists_what_the_model_may_spawn() {
        // The available line ships on every session and had no test.
        let dir = tempfile::tempdir().unwrap();
        let user = dir.path().join("agents");
        write_agent(&user, "scout.md", GOOD);

        let (tools, subagents) = load(LoadRequest {
            agents: rho_skills::AgentConfig {
                user_dirs: vec![user],
                session_root: None,
                project_trusted: false,
                discover: true,
            },
            parent_config: SessionConfig::new(
                "test-model",
                dir.path().to_path_buf(),
                Arc::new(rho_core::AllowAllPolicy),
            ),
            provider: Arc::new(NeverProvider),
            hooks: Arc::new(HookChain::new()),
            parent_tools: rho_tools::builtin_tools(),
            limits: SubagentLimits::new(),
        })
        .await;

        assert!(
            !tools.is_empty(),
            "one definition loaded, so the tools exist"
        );
        assert_eq!(subagents.loaded, 1);
        let joined = subagents.notices.join("\n");
        assert!(
            joined.contains("available to spawn_agent: scout"),
            "the user learns what the model may spawn: {joined}"
        );
    }

    #[tokio::test]
    async fn a_definition_warning_reaches_the_user_too() {
        // A loaded definition may still hold a warning, for example a dropped tool
        // keyword. Nothing printed one, so `tools: all, read` narrowed a child in
        // silence. A skill warning has always printed. See SPEC-definition-rejection
        // section 5.
        let dir = tempfile::tempdir().unwrap();
        write_agent(
            dir.path(),
            "scout.md",
            "---\nname: scout\ndescription: Recon.\ntools: all, read\n---\nbody\n",
        );

        let set = discover_in(dir.path()).await;
        assert_eq!(set.loaded.len(), 1);
        assert!(!set.loaded[0].warnings.is_empty(), "the loader warned");
        let joined = notices_for(&set).join("\n");
        assert!(
            joined.contains("scout"),
            "the notice names the definition: {joined}"
        );
        // Every warning the loader raised must reach the user, whatever it says.
        for warning in &set.loaded[0].warnings {
            assert!(
                joined.contains(warning.as_str()),
                "this warning never reached the user: {warning}"
            );
        }
    }

    #[tokio::test]
    async fn an_untrusted_project_definition_prints_no_warning() {
        // A warning is rho's text with the file's text inside it. A rejection from an
        // untrusted project quotes nothing, and a warning follows the same rule. The
        // definition is not loaded, so its warning changes nothing in this session.
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write_agent(
            &root.join(".rho").join("agents"),
            "sneaky.md",
            "---\nname: sneaky\ndescription: Recon.\ntools: all, rho-is-insecure\n---\nbody\n",
        );
        let config = rho_skills::AgentConfig {
            user_dirs: Vec::new(),
            session_root: Some(root.to_path_buf()),
            project_trusted: false,
            discover: true,
        };
        let set = rho_skills::discover_agents(&config).await;
        assert_eq!(set.withheld.len(), 1, "it parsed, so it is withheld");
        assert!(!set.withheld[0].warnings.is_empty(), "the loader warned");

        let joined = notices_for(&set).join("\n");
        assert!(
            !joined.contains("rho-is-insecure"),
            "an untrusted file puts no prose of its own on a line: {joined}"
        );
        assert!(
            joined.contains("not loaded: sneaky"),
            "the user still learns the file exists: {joined}"
        );

        // With trust, the same warning prints, because the user vouched for the file.
        let trusted = rho_skills::AgentConfig {
            project_trusted: true,
            ..config
        };
        let set = rho_skills::discover_agents(&trusted).await;
        let joined = notices_for(&set).join("\n");
        assert!(
            joined.contains("rho-is-insecure"),
            "a trusted file reports its own warning: {joined}"
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

    // A provider that is never asked to stream. `load` stores the provider in the spawn
    // environment but does not call it, so a stub with an erroring `stream` is enough and no
    // test reaches the network.
    struct StubProvider;

    #[async_trait::async_trait]
    impl Provider for StubProvider {
        fn id(&self) -> &str {
            "stub"
        }
        async fn stream(
            &self,
            _request: rho_core::CompletionRequest,
            _cancel: rho_core::CancelToken,
        ) -> Result<rho_core::ProviderStream, rho_core::ProviderError> {
            Err(rho_core::ProviderError::Decode(
                "the stub never streams".to_string(),
            ))
        }
    }

    fn stub_request(session_root: &std::path::Path, discover: bool) -> LoadRequest {
        let config = SessionConfig::new(
            "m".to_string(),
            session_root.to_path_buf(),
            Arc::new(rho_core::AllowAllPolicy),
        );
        LoadRequest {
            agents: {
                let mut agents =
                    rho_skills::AgentConfig::with_default_user_dirs(session_root.to_path_buf());
                agents.project_trusted = true;
                agents.discover = discover;
                agents
            },
            parent_config: config,
            provider: Arc::new(StubProvider),
            hooks: Arc::new(HookChain::default()),
            parent_tools: Vec::new(),
            limits: SubagentLimits::new(),
        }
    }

    #[tokio::test]
    async fn no_agent_discovery_registers_no_spawn_tool() {
        // D4, the behavioural half. `--no-agents` sets `discover_agents` false, and that must
        // reach `discover` here and remove `spawn_agent`. When discovery is off,
        // `rho_skills::discover_agents` returns before it scans any directory, so this reads
        // no real `~/.rho` or `~/.agents` and its result cannot change per machine. A trusted
        // project definition is present precisely to prove it is the switch, not an empty
        // tree, that yields no tool.
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join(".rho").join("agents");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("scout.md"),
            "---\nname: scout\ndescription: A scout that reads.\n---\nbody\n",
        )
        .unwrap();

        let (tools, subagents) = load(stub_request(root.path(), false)).await;
        assert!(
            tools.is_empty(),
            "agent discovery is off, so no spawn tool may be registered"
        );
        assert_eq!(
            subagents.loaded, 0,
            "a definition present but not discovered must not be loaded"
        );
    }
}
