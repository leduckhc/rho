//! Skill discovery and origin classification.
//!
//! Discovery reads frontmatter only. It classifies each skill as `User` or
//! `Project`, and resolves paths before it classifies, so a symlink from a
//! trusted directory into the session root cannot smuggle a project skill in as
//! trusted.

use crate::frontmatter::{SkillFields, parse_skill_file};
use crate::types::{Skill, SkillConfig, SkillOrigin, SkillSet};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// The source of a scanned directory, before path resolution.
#[derive(Clone, Copy)]
enum Source {
    /// A trusted user directory.
    User,
    /// A project directory under the session root or an ancestor.
    Project,
    /// An explicit path the user named. Always trusted.
    Explicit,
}

/// One file to consider as a skill.
struct Candidate {
    /// The `SKILL.md` file, or a bare `.md` file.
    path: PathBuf,
    /// The directory that holds the skill.
    root: PathBuf,
    /// The source, before resolution.
    source: Source,
}

/// Find every skill. Reads only frontmatter, never a whole body.
pub async fn discover(config: &SkillConfig) -> SkillSet {
    let candidates = gather_candidates(config);
    let canonical_root = config
        .session_root
        .as_deref()
        .and_then(|root| root.canonicalize().ok());

    let mut set = SkillSet::default();
    let mut seen_paths: HashSet<PathBuf> = HashSet::new();
    let mut seen_names: HashSet<String> = HashSet::new();

    for candidate in candidates {
        // Skip a file already seen through another route, for example a symlink
        // that resolves to a file found directly.
        let identity = candidate
            .path
            .canonicalize()
            .unwrap_or_else(|_| candidate.path.clone());
        if !seen_paths.insert(identity) {
            continue;
        }

        let fallback_name = candidate
            .root
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();

        let fields = parse_skill_file(&candidate.path, &fallback_name).await;
        let SkillFields::Load {
            name,
            description,
            model_invocation_disabled,
            warnings,
        } = fields
        else {
            if let SkillFields::Skip { reason } = fields {
                tracing::warn!(path = %candidate.path.display(), "skill does not load: {reason}");
            }
            continue;
        };

        if !seen_names.insert(name.clone()) {
            tracing::warn!(
                skill = %name,
                path = %candidate.path.display(),
                "a skill with this name is already loaded. The first one is kept."
            );
            append_collision_warning(&mut set, &name, &candidate.path);
            continue;
        }

        let origin = classify(&candidate, canonical_root.as_deref());
        let skill = Skill {
            name,
            description,
            path: candidate.path,
            root: candidate.root,
            origin,
            model_invocation_disabled,
            warnings,
        };

        match origin {
            SkillOrigin::User => set.loaded.push(skill),
            SkillOrigin::Project => {
                if config.project_trusted {
                    set.loaded.push(skill);
                } else {
                    set.withheld.push(skill);
                }
            }
        }
    }

    set
}

/// Decide the origin of a candidate. Resolve the path first.
///
/// An explicit path is always `User`, because the user named it. A project
/// directory is always `Project`. A user directory is `User`, unless the
/// resolved path lands inside the session root. That last rule closes the
/// symlink hole: a link in a trusted directory that points into the repository
/// is treated as a project skill.
fn classify(candidate: &Candidate, canonical_root: Option<&Path>) -> SkillOrigin {
    match candidate.source {
        Source::Explicit => SkillOrigin::User,
        Source::Project => SkillOrigin::Project,
        Source::User => {
            if is_inside(&candidate.path, canonical_root) {
                SkillOrigin::Project
            } else {
                SkillOrigin::User
            }
        }
    }
}

/// True when the resolved `path` sits inside the resolved `root`.
///
/// Both sides are canonicalised, so a symlink resolves to its target and the
/// macOS `/var` and `/private/var` pair compares equal.
fn is_inside(path: &Path, canonical_root: Option<&Path>) -> bool {
    let Some(root) = canonical_root else {
        return false;
    };
    match path.canonicalize() {
        Ok(resolved) => resolved.starts_with(root),
        Err(_) => false,
    }
}

/// Add a collision warning to the kept skill with this name.
fn append_collision_warning(set: &mut SkillSet, name: &str, duplicate: &Path) {
    let message = format!(
        "another skill named \"{name}\" was found at {} and ignored. The first one is kept.",
        duplicate.display()
    );
    for skill in set.loaded.iter_mut().chain(set.withheld.iter_mut()) {
        if skill.name == name {
            skill.warnings.push(message);
            return;
        }
    }
}

/// Build the ordered list of candidate files from the config.
///
/// The order follows the discovery table: user directories, then explicit
/// paths, then project directories. An earlier source wins a name collision.
fn gather_candidates(config: &SkillConfig) -> Vec<Candidate> {
    let mut candidates = Vec::new();

    if config.discover {
        for dir in &config.user_dirs {
            collect_dir(dir, Source::User, &mut candidates);
        }
    }

    for path in &config.explicit {
        collect_explicit(path, &mut candidates);
    }

    if config.discover
        && let Some(root) = &config.session_root
    {
        collect_dir(
            &root.join(".rho").join("skills"),
            Source::Project,
            &mut candidates,
        );
        for dir in agents_dirs_up_to_repo_root(root) {
            collect_dir(&dir, Source::Project, &mut candidates);
        }
    }

    candidates
}

/// Collect skills from one directory tree.
///
/// A directory with a `SKILL.md` is a skill, and the walk does not descend into
/// it. A bare `.md` file at the top level is a skill, unless the directory is an
/// `.agents` directory, which is shared with other harnesses.
fn collect_dir(dir: &Path, source: Source, out: &mut Vec<Candidate>) {
    if !dir.is_dir() {
        return;
    }
    let allow_bare_md = !has_component(dir, ".agents");
    let mut stack = vec![(dir.to_path_buf(), true)];

    while let Some((current, is_top)) = stack.pop() {
        let skill_md = current.join("SKILL.md");
        if skill_md.is_file() {
            out.push(Candidate {
                path: skill_md,
                root: current,
                source,
            });
            continue;
        }

        let Ok(entries) = std::fs::read_dir(&current) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push((path, false));
            } else if is_top && allow_bare_md && is_bare_md(&path) {
                out.push(Candidate {
                    path: path.clone(),
                    root: current.clone(),
                    source,
                });
            }
        }
    }
}

/// Collect an explicit path. It may be a directory with a `SKILL.md`, or a file.
fn collect_explicit(path: &Path, out: &mut Vec<Candidate>) {
    if path.is_dir() {
        let skill_md = path.join("SKILL.md");
        if skill_md.is_file() {
            out.push(Candidate {
                path: skill_md,
                root: path.to_path_buf(),
                source: Source::Explicit,
            });
        }
    } else if path.is_file() {
        let root = path.parent().map(Path::to_path_buf).unwrap_or_default();
        out.push(Candidate {
            path: path.to_path_buf(),
            root,
            source: Source::Explicit,
        });
    }
}

/// The `.agents/skills` directories from the session root up to the repository
/// root. The walk stops at the first directory that holds a `.git` entry.
fn agents_dirs_up_to_repo_root(root: &Path) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    let mut current = Some(root);
    while let Some(dir) = current {
        dirs.push(dir.join(".agents").join("skills"));
        if dir.join(".git").exists() {
            break;
        }
        current = dir.parent();
    }
    dirs
}

/// True when a `.md` file is not a `SKILL.md`.
fn is_bare_md(path: &Path) -> bool {
    path.extension().and_then(|ext| ext.to_str()) == Some("md")
        && path.file_name().and_then(|name| name.to_str()) != Some("SKILL.md")
}

/// True when any component of a path equals `name`.
fn has_component(path: &Path, name: &str) -> bool {
    path.components()
        .any(|component| component.as_os_str() == name)
}
