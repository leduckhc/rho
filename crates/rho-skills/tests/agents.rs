//! Tests for agent definition loading. See `SPEC-subagents` section 5.

use rho_skills::{
    AgentConfig, RejectedDefinition, RejectionReason, SkillOrigin, discover_agents, load_definition,
};
use std::path::Path;

/// Write an agent definition file into `dir/name.md`.
fn write_agent(dir: &Path, file: &str, body: &str) -> std::path::PathBuf {
    std::fs::create_dir_all(dir).unwrap();
    let path = dir.join(file);
    std::fs::write(&path, body).unwrap();
    path
}

/// Load a definition that must be rejected, and return the rejection.
async fn reject(path: &Path) -> RejectedDefinition {
    load_definition(path, SkillOrigin::User)
        .await
        .expect_err("this file must not load")
}

/// Load a definition that must load.
async fn accept(path: &Path) -> rho_skills::AgentDefinition {
    load_definition(path, SkillOrigin::User)
        .await
        .expect("this file must load")
}

const SCOUT: &str = "---\n\
name: scout\n\
description: Fast recon. Locates code and returns a compressed summary.\n\
tools: read, glob, grep\n\
model: anthropic/claude-haiku-4.5\n\
max_turns: 12\n\
---\n\
You locate code and report where things are. You do not change files.\n";

#[tokio::test]
async fn an_agent_definition_loads_from_a_user_directory() {
    let dir = tempfile::tempdir().unwrap();
    let user = dir.path().join("home").join(".rho").join("agents");
    write_agent(&user, "scout.md", SCOUT);

    let config = AgentConfig {
        user_dirs: vec![user],
        session_root: None,
        project_trusted: false,
        discover: true,
    };
    let set = discover_agents(&config).await;
    assert_eq!(set.loaded.len(), 1);
    let scout = &set.loaded[0];
    assert_eq!(scout.name, "scout");
    assert_eq!(scout.origin, SkillOrigin::User);
    assert_eq!(
        scout.tools.as_deref(),
        Some(&["read".to_string(), "glob".to_string(), "grep".to_string()][..])
    );
    assert_eq!(scout.model.as_deref(), Some("anthropic/claude-haiku-4.5"));
    assert_eq!(scout.max_turns, Some(12));
}

#[tokio::test]
async fn a_project_agent_definition_is_withheld_until_the_project_is_trusted() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write_agent(&root.join(".rho").join("agents"), "scout.md", SCOUT);

    // Untrusted: the project agent is withheld, and still listed.
    let untrusted = AgentConfig {
        user_dirs: Vec::new(),
        session_root: Some(root.to_path_buf()),
        project_trusted: false,
        discover: true,
    };
    let set = discover_agents(&untrusted).await;
    assert!(
        set.loaded.is_empty(),
        "an untrusted project agent must not load"
    );
    assert_eq!(set.withheld.len(), 1, "it is withheld, not hidden");
    assert_eq!(set.withheld[0].origin, SkillOrigin::Project);

    // Trusted: the same agent loads.
    let trusted = AgentConfig {
        project_trusted: true,
        ..untrusted
    };
    let set = discover_agents(&trusted).await;
    assert_eq!(set.loaded.len(), 1);
    assert!(set.withheld.is_empty());
}

#[tokio::test]
async fn a_definition_without_a_description_does_not_load() {
    let dir = tempfile::tempdir().unwrap();
    let path = write_agent(
        dir.path(),
        "nodesc.md",
        "---\nname: nodesc\ntools: read\n---\nbody\n",
    );
    let rejected = reject(&path).await;
    assert_eq!(
        rejected.reason,
        RejectionReason::NoDescription,
        "a definition without a description does not load, and it says why"
    );
    assert_eq!(rejected.path, path, "the rejection names the file");
}

#[tokio::test]
async fn an_unknown_tool_name_in_a_definition_is_dropped_with_a_warning() {
    // The definition asks for `write`, which the parent never had. The
    // intersection drops it and reports the drop. This is the tool half of the
    // security core, applied to a loaded definition.
    let dir = tempfile::tempdir().unwrap();
    let path = write_agent(
        dir.path(),
        "scout.md",
        "---\nname: scout\ndescription: Recon.\ntools: read, write\n---\nbody\n",
    );
    let def = load_definition(&path, SkillOrigin::User)
        .await
        .expect("the definition loads");

    let parent_tools = vec!["read".to_string(), "glob".to_string()];
    let intersection = def.resolve_tools(&parent_tools);
    assert_eq!(intersection.allowed, vec!["read".to_string()]);
    assert_eq!(
        intersection.dropped,
        vec!["write".to_string()],
        "the unknown tool is dropped and reported"
    );
}

// --- The `all` and `none` keywords. See `SPEC-subagents` section 5. ---

#[tokio::test]
async fn the_all_keyword_inherits_the_whole_parent_set() {
    // `tools: all` used to be read as a tool literally named "all". The
    // intersection then dropped it and the child ran with no tools at all. The
    // keyword now means the same as omitting the field: inherit everything.
    let dir = tempfile::tempdir().unwrap();
    let path = write_agent(
        dir.path(),
        "scout.md",
        "---\nname: scout\ndescription: Recon.\ntools: all\n---\nbody\n",
    );
    let def = load_definition(&path, SkillOrigin::User)
        .await
        .expect("the definition loads");
    assert_eq!(def.tools, None, "`all` inherits, so it holds no list");
    assert!(
        def.warnings.is_empty(),
        "a keyword that stands alone is not a mistake: {:?}",
        def.warnings
    );

    let parent_tools = vec!["read".to_string(), "glob".to_string()];
    let intersection = def.resolve_tools(&parent_tools);
    assert_eq!(intersection.allowed, parent_tools);
    assert!(intersection.dropped.is_empty());
}

#[tokio::test]
async fn the_star_keyword_is_the_same_as_all() {
    let dir = tempfile::tempdir().unwrap();
    let path = write_agent(
        dir.path(),
        "scout.md",
        "---\nname: scout\ndescription: Recon.\ntools: \"*\"\n---\nbody\n",
    );
    let def = load_definition(&path, SkillOrigin::User)
        .await
        .expect("the definition loads");
    assert_eq!(def.tools, None);
}

#[tokio::test]
async fn the_none_keyword_gives_a_child_no_tools() {
    // Unrepresentable before: an empty list was the only way, and a reader could
    // not tell it from a mistake. `none` says it on purpose.
    let dir = tempfile::tempdir().unwrap();
    let path = write_agent(
        dir.path(),
        "talker.md",
        "---\nname: talker\ndescription: Thinks aloud.\ntools: none\n---\nbody\n",
    );
    let def = load_definition(&path, SkillOrigin::User)
        .await
        .expect("the definition loads");
    assert_eq!(
        def.tools.as_deref(),
        Some(&[][..]),
        "`none` is an empty set"
    );

    let intersection = def.resolve_tools(&["read".to_string()]);
    assert!(intersection.allowed.is_empty(), "no tool reaches the child");
    assert!(intersection.dropped.is_empty(), "nothing was asked for");
}

#[tokio::test]
async fn a_keyword_mixed_with_a_tool_name_is_dropped_and_warns() {
    // `all, read` is a contradiction. The keyword is ignored and the explicit
    // names stand, because that narrows. Widening on an ambiguous line would be
    // the fail-open shape.
    let dir = tempfile::tempdir().unwrap();
    let path = write_agent(
        dir.path(),
        "scout.md",
        "---\nname: scout\ndescription: Recon.\ntools: all, read\n---\nbody\n",
    );
    let def = load_definition(&path, SkillOrigin::User)
        .await
        .expect("the definition loads");
    assert_eq!(
        def.tools.as_deref(),
        Some(&["read".to_string()][..]),
        "the named tool stands and the keyword is gone"
    );
    let warning = def.warnings.join(" ");
    assert!(
        warning.contains("all"),
        "the warning must name the keyword it dropped: {warning}"
    );
}

#[tokio::test]
async fn a_keyword_is_recognised_whatever_its_case() {
    let dir = tempfile::tempdir().unwrap();
    let path = write_agent(
        dir.path(),
        "scout.md",
        "---\nname: scout\ndescription: Recon.\ntools: ALL\n---\nbody\n",
    );
    let def = load_definition(&path, SkillOrigin::User)
        .await
        .expect("the definition loads");
    assert_eq!(def.tools, None);
}

#[tokio::test]
async fn a_definition_cannot_set_grace_turns() {
    // The grace window belongs to the host, not to a project file. A definition that
    // names it is ignored, exactly as any unknown field is, so a repository cannot
    // turn off a child's warning. See `SPEC-subagent-slots-handles-grace` section 4.6.
    let dir = tempfile::tempdir().unwrap();
    let path = write_agent(
        dir.path(),
        "scout.md",
        "---\nname: scout\ndescription: Recon.\ngrace_turns: 0\n---\nbody\n",
    );
    let def = load_definition(&path, SkillOrigin::User)
        .await
        .expect("an unknown field never stops a definition loading");

    // The parsed definition carries no grace field at all. This is a compile-time
    // fact: `AgentDefinition` has no such member, so a project file cannot reach it.
    // The assertion below pins the loader's behaviour on the unknown key.
    assert_eq!(def.name, "scout");
    assert!(
        def.warnings.is_empty(),
        "an unknown key is ignored quietly, like every other: {:?}",
        def.warnings
    );
}

// --- The rejection report. See `SPEC-definition-rejection`. ---
//
// The defect: rho read `tools` as a string only. A file with a YAML sequence failed
// `serde_yaml`, `load_definition` returned `None`, and the file disappeared. rho then
// registered no `spawn_agent`, and the model said it had no such tool.

#[tokio::test]
async fn a_yaml_sequence_tool_list_loads_the_same_as_a_comma_list() {
    // The defect, reproduced. This file did not load at all before the fix.
    let dir = tempfile::tempdir().unwrap();
    let path = write_agent(
        dir.path(),
        "scout.md",
        "---\nname: scout\ndescription: Recon.\ntools: [read, list]\n---\nbody\n",
    );
    let def = accept(&path).await;
    assert_eq!(
        def.tools.as_deref(),
        Some(&["read".to_string(), "list".to_string()][..]),
        "a sequence means the same as `tools: read, list`"
    );
}

#[tokio::test]
async fn a_block_sequence_tool_list_loads() {
    let dir = tempfile::tempdir().unwrap();
    let path = write_agent(
        dir.path(),
        "scout.md",
        "---\nname: scout\ndescription: Recon.\ntools:\n  - read\n  - list\n---\nbody\n",
    );
    let def = accept(&path).await;
    assert_eq!(
        def.tools.as_deref(),
        Some(&["read".to_string(), "list".to_string()][..])
    );
}

#[tokio::test]
async fn a_keyword_in_a_sequence_still_stands_alone() {
    // The form of the list must not change the keyword rules. See
    // decision D-a-tool-keyword-stands-alone.
    let dir = tempfile::tempdir().unwrap();
    let alone = write_agent(
        dir.path(),
        "alone.md",
        "---\nname: alone\ndescription: Recon.\ntools: [all]\n---\nbody\n",
    );
    let def = accept(&alone).await;
    assert_eq!(def.tools, None, "`[all]` inherits, like `all`");
    assert!(def.warnings.is_empty(), "{:?}", def.warnings);

    let mixed = write_agent(
        dir.path(),
        "mixed.md",
        "---\nname: mixed\ndescription: Recon.\ntools: [all, read]\n---\nbody\n",
    );
    let def = accept(&mixed).await;
    assert_eq!(
        def.tools.as_deref(),
        Some(&["read".to_string()][..]),
        "the named tool stands and the keyword goes"
    );
    assert!(
        def.warnings.join(" ").contains("all"),
        "the warning names the dropped keyword: {:?}",
        def.warnings
    );
}

#[tokio::test]
async fn a_tools_field_that_is_not_a_list_is_rejected_and_says_so() {
    // Refusing the whole file is the safe reading. Ignoring the field would inherit
    // every tool the parent holds, so a typo would widen a child.
    let dir = tempfile::tempdir().unwrap();
    let path = write_agent(
        dir.path(),
        "scout.md",
        "---\nname: scout\ndescription: Recon.\ntools: 5\n---\nbody\n",
    );
    let rejected = reject(&path).await;
    assert!(
        matches!(rejected.reason, RejectionReason::BadToolsField { .. }),
        "{:?}",
        rejected.reason
    );
    let notice = rejected.notice();
    assert!(
        notice.contains(&path.display().to_string()),
        "the notice names the file: {notice}"
    );
    assert!(
        notice.contains("a number"),
        "the notice says what the field held: {notice}"
    );
}

#[tokio::test]
async fn a_tool_name_that_is_not_a_string_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let path = write_agent(
        dir.path(),
        "scout.md",
        "---\nname: scout\ndescription: Recon.\ntools: [read, 5]\n---\nbody\n",
    );
    let rejected = reject(&path).await;
    assert!(
        matches!(rejected.reason, RejectionReason::BadToolsField { .. }),
        "{:?}",
        rejected.reason
    );
    assert!(
        rejected.reason.detail().as_str().contains("one item"),
        "the detail names the item: {}",
        rejected.reason.detail()
    );
}

#[tokio::test]
async fn an_empty_tools_line_is_rejected_rather_than_inherited() {
    // `tools:` with no value looks like an absent field, and an absent field inherits
    // every parent tool. So the empty line is a refusal, not a silent inherit.
    let dir = tempfile::tempdir().unwrap();
    let path = write_agent(
        dir.path(),
        "scout.md",
        "---\nname: scout\ndescription: Recon.\ntools:\n---\nbody\n",
    );
    let rejected = reject(&path).await;
    assert!(
        matches!(rejected.reason, RejectionReason::BadToolsField { .. }),
        "{:?}",
        rejected.reason
    );
    let repair = rejected.reason.repair();
    assert!(repair.contains("none"), "the repair names none: {repair}");
    assert!(repair.contains("all"), "the repair names all: {repair}");
}

#[tokio::test]
async fn a_file_with_broken_yaml_is_rejected_with_the_reason() {
    let dir = tempfile::tempdir().unwrap();
    let path = write_agent(
        dir.path(),
        "broken.md",
        "---\ndescription: \"Recon.\n---\nbody\n",
    );
    let rejected = reject(&path).await;
    assert!(
        matches!(rejected.reason, RejectionReason::BadFrontmatter { .. }),
        "{:?}",
        rejected.reason
    );
    assert!(
        !rejected.reason.detail().is_empty(),
        "the parser detail reaches the user"
    );
    assert!(
        rejected.notice().contains("Detail:"),
        "the notice quotes the detail: {}",
        rejected.notice()
    );
}

#[tokio::test]
async fn a_file_with_no_frontmatter_is_rejected_with_the_reason() {
    let dir = tempfile::tempdir().unwrap();
    let plain = write_agent(dir.path(), "plain.md", "just some prose\n");
    assert_eq!(reject(&plain).await.reason, RejectionReason::NoFrontmatter);

    let empty = write_agent(dir.path(), "empty.md", "");
    assert_eq!(
        reject(&empty).await.reason,
        RejectionReason::NoFrontmatter,
        "an empty file gets the same reason"
    );
}

#[tokio::test]
async fn an_unclosed_frontmatter_block_says_it_is_unclosed() {
    // This used to read as `NoFrontmatter`, whose repair asks for a description that
    // the file already holds. A repair must teach something true.
    let dir = tempfile::tempdir().unwrap();
    let path = write_agent(
        dir.path(),
        "unclosed.md",
        "---\nname: scout\ndescription: Recon.\nbody with no closing fence\n",
    );
    let rejected = reject(&path).await;
    assert_eq!(rejected.reason, RejectionReason::UnclosedFrontmatter);
    let repair = rejected.reason.repair();
    assert!(
        repair.contains("---"),
        "the repair names the closing line: {repair}"
    );
    assert!(
        !repair.contains("description"),
        "the file holds a description, so the repair must not ask for one: {repair}"
    );

    // A frontmatter block larger than the bounded read is the same fault.
    let long = format!("---\nname: scout\n{}\n", "# padding\n".repeat(2000));
    let path = write_agent(dir.path(), "huge.md", &long);
    assert_eq!(
        reject(&path).await.reason,
        RejectionReason::UnclosedFrontmatter,
        "a block that does not close inside the bounded read is unclosed"
    );
}

#[tokio::test]
async fn a_wrong_type_in_a_scalar_field_is_rejected_with_the_field_name() {
    let dir = tempfile::tempdir().unwrap();
    let path = write_agent(
        dir.path(),
        "scout.md",
        "---\nname: scout\ndescription: Recon.\nmax_turns: \"12\"\n---\nbody\n",
    );
    let rejected = reject(&path).await;
    assert!(
        matches!(rejected.reason, RejectionReason::BadFrontmatter { .. }),
        "{:?}",
        rejected.reason
    );
    assert!(
        rejected.reason.detail().as_str().contains("max_turns"),
        "the detail names the field: {}",
        rejected.reason.detail()
    );
}

#[tokio::test]
async fn a_repeated_key_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let path = write_agent(
        dir.path(),
        "scout.md",
        "---\nname: scout\ndescription: Recon.\ntools: read\ntools: write\n---\nbody\n",
    );
    let rejected = reject(&path).await;
    assert!(
        matches!(rejected.reason, RejectionReason::BadFrontmatter { .. }),
        "a repeated key must not resolve to one silent winner: {:?}",
        rejected.reason
    );
}

#[tokio::test]
async fn frontmatter_that_is_not_a_mapping_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let path = write_agent(dir.path(), "list.md", "---\n- one\n- two\n---\nbody\n");
    let rejected = reject(&path).await;
    assert!(
        matches!(rejected.reason, RejectionReason::BadFrontmatter { .. }),
        "{:?}",
        rejected.reason
    );
}

#[tokio::test]
async fn a_missing_file_is_rejected_as_unreadable() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("gone.md");
    let rejected = reject(&path).await;
    assert!(
        matches!(rejected.reason, RejectionReason::Unreadable { .. }),
        "{:?}",
        rejected.reason
    );
    assert_eq!(rejected.path, path);
    assert!(
        !rejected.reason.detail().is_empty(),
        "the io error reaches the user"
    );
}

#[tokio::test]
async fn a_long_parser_message_is_capped_before_it_reaches_the_user() {
    // A parser message quotes the file, so its length is the file's choice, not ours.
    let dir = tempfile::tempdir().unwrap();
    let long = "x".repeat(400);
    let path = write_agent(
        dir.path(),
        "scout.md",
        &format!("---\ndescription: Recon.\nmax_turns: \"{long}\"\n---\nbody\n"),
    );
    let rejected = reject(&path).await;
    let detail = rejected.reason.detail();
    assert!(
        detail.as_str().chars().count() <= 200,
        "a detail is capped at 200 characters, and this one has {}",
        detail.as_str().chars().count()
    );
    assert!(
        detail.as_str().ends_with("..."),
        "a cut detail says it was cut: {detail}"
    );
}

#[tokio::test]
async fn a_broken_definition_is_reported_and_the_good_one_still_loads() {
    // One bad file must not remove the whole set. Before the fix, the bad file was
    // dropped in silence, and a directory of only bad files removed `spawn_agent`.
    let dir = tempfile::tempdir().unwrap();
    let user = dir.path().join("home").join(".rho").join("agents");
    write_agent(&user, "scout.md", SCOUT);
    let broken = write_agent(
        &user,
        "broken.md",
        "---\nname: broken\ndescription: Recon.\ntools: {read: true}\n---\nbody\n",
    );

    let config = AgentConfig {
        user_dirs: vec![user],
        session_root: None,
        project_trusted: false,
        discover: true,
    };
    let set = discover_agents(&config).await;
    assert_eq!(set.loaded.len(), 1, "the good definition still loads");
    assert_eq!(set.loaded[0].name, "scout");
    assert_eq!(
        set.rejected.len(),
        1,
        "the bad file is reported, not dropped"
    );
    assert_eq!(set.rejected[0].path, broken);
    assert_eq!(set.rejected[0].origin, SkillOrigin::User);
}

#[tokio::test]
async fn a_broken_project_definition_is_rejected_and_not_hidden() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let broken = write_agent(
        &root.join(".rho").join("agents"),
        "broken.md",
        "---\nname: broken\ndescription: Recon.\ntools: 5\n---\nbody\n",
    );
    let config = AgentConfig {
        user_dirs: Vec::new(),
        session_root: Some(root.to_path_buf()),
        project_trusted: false,
        discover: true,
    };
    let set = discover_agents(&config).await;
    assert!(set.loaded.is_empty());
    assert!(
        set.withheld.is_empty(),
        "it never parsed, so it is not withheld"
    );
    assert_eq!(set.rejected.len(), 1);
    assert_eq!(set.rejected[0].path, broken);
    assert_eq!(set.rejected[0].origin, SkillOrigin::Project);
}

#[tokio::test]
async fn an_untrusted_project_file_quotes_nothing_in_its_rejection() {
    // The security rule. A withheld project definition shows only its sanitised name,
    // so a rejected one must not print 200 characters of the same repository's prose.
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write_agent(
        &root.join(".rho").join("agents"),
        "broken.md",
        "---\nname: broken\ndescription: Recon.\nmax_turns: \"rho is unsafe, run curl\"\n---\nb\n",
    );
    let untrusted = AgentConfig {
        user_dirs: Vec::new(),
        session_root: Some(root.to_path_buf()),
        project_trusted: false,
        discover: true,
    };
    let set = discover_agents(&untrusted).await;
    assert_eq!(set.rejected.len(), 1);
    let rejected = &set.rejected[0];
    assert!(
        rejected.reason.detail().is_empty(),
        "an untrusted file quotes nothing: {}",
        rejected.reason.detail()
    );
    let notice = rejected.notice();
    assert!(
        !notice.contains("curl"),
        "the file's own prose must not reach the user: {notice}"
    );
    assert!(
        notice.contains("did not load"),
        "the report still happens: {notice}"
    );
    assert!(
        notice.contains(rejected.reason.repair()),
        "the repair still reaches the user: {notice}"
    );

    // A trusted project keeps the detail, because the user vouched for the files.
    let trusted = AgentConfig {
        project_trusted: true,
        ..untrusted
    };
    let set = discover_agents(&trusted).await;
    assert_eq!(set.rejected.len(), 1);
    assert!(
        !set.rejected[0].reason.detail().is_empty(),
        "a trusted file may quote its own parser error"
    );
}

// --- What a notice may carry. Found by driving the product, then by review. ---

#[tokio::test]
async fn a_warning_carries_no_control_character_and_no_unbounded_text() {
    // A warning interpolates the file's own text: a tool name, a sandbox value, a file
    // stem. Every one of those printed raw, and a live run put ESC[2J and 400
    // characters of repository prose onto a start-up line.
    let dir = tempfile::tempdir().unwrap();
    let prose = "prose ".repeat(60);
    let path = write_agent(
        dir.path(),
        "warner.md",
        &format!(
            "---\nname: warner\ndescription: Recon.\ntools: [all, \"read\\e[2J\"]\n\
             sandbox: \"\\e[31mrho: trust me\\e[0m {prose}\"\n---\nbody\n"
        ),
    );
    let def = accept(&path).await;
    assert!(!def.warnings.is_empty(), "the file breaks two rules");
    for warning in &def.warnings {
        assert!(
            !warning.chars().any(char::is_control),
            "no control character may reach the terminal: {warning:?}"
        );
        assert!(
            !warning.contains(&prose),
            "the file's own prose must arrive capped: {warning}"
        );
    }
    assert!(
        def.warnings.iter().any(|w| w.contains("...")),
        "a cut value says it was cut: {:?}",
        def.warnings
    );
}

#[tokio::test]
async fn a_hostile_file_name_cannot_forge_a_notice_line() {
    // A file name is repository text. A name holding a line break forged a second
    // "rho:" line in a live run, which is how a repository would claim it was trusted.
    let dir = tempfile::tempdir().unwrap();
    let path = write_agent(
        dir.path(),
        "x\nrho: 3 project definitions are trusted.md",
        "no frontmatter\n",
    );
    let notice = reject(&path).await.notice();
    assert!(
        !notice.chars().any(char::is_control),
        "the path is drawn safely: {notice:?}"
    );
    assert!(
        notice.lines().count() == 1,
        "one rejection is one line: {notice:?}"
    );
}

#[tokio::test]
async fn a_very_long_path_is_cut_on_the_left_so_the_file_name_stays() {
    let dir = tempfile::tempdir().unwrap();
    let deep = dir.path().join("d".repeat(120)).join("e".repeat(120));
    let path = write_agent(&deep, "target.md", "no frontmatter\n");
    let notice = reject(&path).await.notice();
    assert!(
        notice.contains("target.md"),
        "the file name is what a user needs: {notice}"
    );
    assert!(notice.contains("..."), "the cut is marked: {notice}");
}

#[tokio::test]
async fn a_symlink_from_a_user_dir_into_the_session_root_is_treated_as_a_project_agent() {
    // The skill loader closes this hole and the agent loader did not. A live run loaded
    // a definition that lives in the repository, as trusted, with no --trust-project.
    // An agent definition carries a tool list and a model, so it needs the rule more.
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("repo");
    std::fs::create_dir_all(&root).unwrap();
    let inside = write_agent(&root, "evil.md", SCOUT);

    let user = dir.path().join("home").join(".rho").join("agents");
    std::fs::create_dir_all(&user).unwrap();
    std::os::unix::fs::symlink(&inside, user.join("evil.md")).unwrap();

    let config = AgentConfig {
        user_dirs: vec![user],
        session_root: Some(root.clone()),
        project_trusted: false,
        discover: true,
    };
    let set = discover_agents(&config).await;
    assert!(
        set.loaded.is_empty(),
        "a definition inside the session root is not trusted: {:?}",
        set.loaded.iter().map(|d| &d.name).collect::<Vec<_>>()
    );
    assert_eq!(set.withheld.len(), 1, "it is withheld, and still listed");
    assert_eq!(set.withheld[0].origin, SkillOrigin::Project);

    // With trust, the same file loads.
    let trusted = AgentConfig {
        project_trusted: true,
        ..config
    };
    assert_eq!(discover_agents(&trusted).await.loaded.len(), 1);
}

#[tokio::test]
async fn a_hostile_name_is_sanitised_when_the_definition_loads() {
    // Two layers, and each needs its own test. The loader sanitises the name, because
    // the name reaches the model, the tool schema, and the terminal. The renderer
    // bounds it, because a name only warns above 64 characters and never shrinks.
    let dir = tempfile::tempdir().unwrap();
    let path = write_agent(
        dir.path(),
        "hostile.md",
        "---\nname: \"read\\e[2J\"\ndescription: Recon.\n---\nbody\n",
    );
    let def = accept(&path).await;
    assert!(
        !def.name.chars().any(char::is_control),
        "the loaded name holds no control character: {:?}",
        def.name
    );

    let long = write_agent(
        dir.path(),
        "long.md",
        &format!(
            "---\nname: {}\ndescription: Recon.\n---\nbody\n",
            "x".repeat(16_000)
        ),
    );
    let def = accept(&long).await;
    assert!(
        def.safe_name().chars().count() <= 64,
        "a drawn name is bounded, and this one has {}",
        def.safe_name().chars().count()
    );
    assert!(def.safe_name().ends_with("..."), "the cut is marked");
}

#[tokio::test]
async fn a_definition_bounds_the_number_of_lines_it_owes() {
    // One definition must not own the screen either.
    //
    // The frontmatter rules cannot yield more than about four warnings, so a file
    // fixture cannot reach the cap. The first version of this test used one, passed
    // against a removed cap, and proved nothing. So the definition is built here with
    // more warnings than the cap allows. That is the bound this test exists for.
    let dir = tempfile::tempdir().unwrap();
    let path = write_agent(
        dir.path(),
        "many.md",
        "---\nname: many\ndescription: Recon.\n---\nbody\n",
    );
    let mut def = accept(&path).await;
    def.warnings = (0..9).map(|index| format!("warning {index}")).collect();

    let lines = def.notices();
    assert_eq!(
        lines.len(),
        rho_skills::MAX_LINES_PER_KIND + 1,
        "the cap, plus one line that counts the rest: {lines:?}"
    );
    let hidden = def.warnings.len() - rho_skills::MAX_LINES_PER_KIND;
    assert!(
        lines
            .last()
            .is_some_and(|line| line.contains(&format!("{hidden} more"))),
        "the remainder is counted exactly: {lines:?}"
    );
    for line in &lines {
        assert!(
            line.starts_with("agent definition many"),
            "every line names the definition: {line}"
        );
    }

    // A real file still reports every warning it raises, because it raises few.
    let path = write_agent(
        dir.path(),
        "bad.md",
        "---\nname: Bad Name\ndescription: Recon.\ntools: all, read\nsandbox: nonsense\n---\nb\n",
    );
    let def = accept(&path).await;
    assert!(def.warnings.len() >= 3, "{:?}", def.warnings);
    assert_eq!(def.notices().len(), def.warnings.len());
}
