//! The prompt block from SPEC-skills section 6.

mod common;

use rho_skills::{Skill, SkillOrigin, prompt_block};
use std::path::PathBuf;

/// Build a skill in memory, without a filesystem.
fn skill(name: &str, disabled: bool) -> Skill {
    Skill {
        name: name.to_string(),
        description: format!("Does {name}."),
        path: PathBuf::from(format!("/skills/{name}/SKILL.md")),
        root: PathBuf::from(format!("/skills/{name}")),
        origin: SkillOrigin::User,
        model_invocation_disabled: disabled,
        warnings: Vec::new(),
    }
}

#[test]
fn prompt_block_lists_only_loaded_skills() {
    let loaded = vec![skill("alpha", false), skill("beta", false)];
    let block = prompt_block(&loaded);

    assert!(block.contains("<name>alpha</name>"));
    assert!(block.contains("<name>beta</name>"));
    assert!(block.contains("<available_skills>"));
    assert!(block.contains("<location>/skills/alpha/SKILL.md</location>"));
}

#[test]
fn prompt_block_omits_a_skill_with_model_invocation_disabled() {
    let loaded = vec![skill("visible", false), skill("hidden", true)];
    let block = prompt_block(&loaded);

    assert!(block.contains("<name>visible</name>"));
    assert!(
        !block.contains("<name>hidden</name>"),
        "a disabled skill is omitted"
    );
}

#[test]
fn prompt_block_omits_a_withheld_skill() {
    // A caller passes only the loaded skills. A withheld skill never reaches
    // the prompt block.
    let loaded = vec![skill("shown", false)];
    let block = prompt_block(&loaded);

    assert!(block.contains("<name>shown</name>"));
    assert!(!block.contains("withheld"));
}

#[test]
fn prompt_block_is_stable_for_the_same_input() {
    // The block sits in the cached prefix, so it must be byte-identical, even
    // when the input order changes.
    let ordered = vec![skill("alpha", false), skill("beta", false)];
    let reversed = vec![skill("beta", false), skill("alpha", false)];

    assert_eq!(prompt_block(&ordered), prompt_block(&reversed));
    assert_eq!(prompt_block(&ordered), prompt_block(&ordered));
}
