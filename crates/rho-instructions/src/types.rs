//! Public data types for project instruction discovery.
//!
//! See `docs/specs/20260821-131725-SPEC-project-instructions.md` section 5. This module
//! holds data only. The discovery logic lives in `discover`, and the rendering lives in
//! `prompt`.

use std::path::{Path, PathBuf};

/// Where an instruction file came from. This decides how rho marks it, and it never
/// decides authority. See D-project-instructions-are-authority-inert.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InstructionOrigin {
    /// A directory the user owns, outside the session root.
    User,
    /// The session root or an ancestor of it. Untrusted input.
    Project,
}

impl InstructionOrigin {
    /// The word rho renders in the block. The model reads this, so it never changes.
    pub fn label(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Project => "project",
        }
    }
}

/// Why rho did not deliver a candidate file. Every variant is reported, never dropped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OmissionReason {
    /// The home directory could not be resolved.
    HomeUnavailable,
    /// The session root is not below the home directory, so no ancestor was walked.
    RootOutsideHome,
    /// The path escaped the directory rho expected it in.
    UnsafePath,
    /// The file could not be read.
    Unreadable,
    /// The file is a symlink. See spec section 3.
    Symlink,
    /// The file is not a regular file.
    NonRegular,
    /// The whole set exceeded `instructions_total_bytes`, so this file was dropped.
    TotalBudget,
    /// The ancestor cap was reached before this directory was walked.
    AncestorCap,
    /// The caller set no session root, so rho walked no project tree. See spec section 2.
    NoSessionRoot,
}

impl OmissionReason {
    /// Plain words for the user. `InstructionSet::notices` builds a line from this.
    pub fn label(self) -> &'static str {
        match self {
            Self::HomeUnavailable => "the home directory could not be resolved",
            Self::RootOutsideHome => "the session root is not below the home directory",
            Self::UnsafePath => "the path escaped its directory",
            Self::Unreadable => "the file could not be read",
            Self::Symlink => "the file is a symlink",
            Self::NonRegular => "the file is not a regular file",
            Self::TotalBudget => "the instruction set reached its total byte budget",
            Self::AncestorCap => "the ancestor directory cap was reached",
            Self::NoSessionRoot => "no session root was set",
        }
    }
}

/// One candidate rho did not deliver, and the reason.
#[derive(Clone, Debug, PartialEq)]
pub struct Omission {
    /// The path, or the name of the missing input for `HomeUnavailable`.
    pub source: String,
    pub reason: OmissionReason,
}

/// One instruction file rho delivered.
#[derive(Clone, Debug, PartialEq)]
pub struct Instruction {
    /// The absolute, canonical path rho read.
    pub path: PathBuf,
    /// The bytes rho kept. Truncated on a character boundary when the file was over budget.
    pub body: String,
    pub origin: InstructionOrigin,
    /// The file size on disk. Larger than `body.len()` when rho truncated.
    pub observed_bytes: usize,
    /// True when `body` holds less than the whole file.
    pub truncated: bool,
}

/// What one gather pass produced.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct InstructionSet {
    /// Delivered files, ordered broad to narrow. The last one wins a conflict.
    pub delivered: Vec<Instruction>,
    /// Every candidate rho refused or dropped, with its reason.
    pub omissions: Vec<Omission>,
}

impl InstructionSet {
    /// True when nothing was delivered and nothing was refused.
    pub fn is_empty(&self) -> bool {
        self.delivered.is_empty() && self.omissions.is_empty()
    }

    /// One line per omission, for the user. This never reaches the model.
    ///
    /// The line names the source and the reason in plain words. An empty set yields an
    /// empty vector. A caller prints these once, before the session starts, exactly as it
    /// prints the skill notices. Returning nothing here would re-ship the defect
    /// D-rho-reads-agents-md forbids, which is silence about a dropped instruction.
    pub fn notices(&self) -> Vec<String> {
        self.omissions
            .iter()
            .map(|omission| {
                format!(
                    "project instructions: skipped {} because {}",
                    omission.source,
                    omission.reason.label()
                )
            })
            .collect()
    }
}

/// The byte and count bounds from spec section 4.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InstructionLimits {
    pub instruction_file_bytes: usize,
    pub instructions_total_bytes: usize,
    pub instruction_ancestor_cap: usize,
}

impl Default for InstructionLimits {
    fn default() -> Self {
        Self {
            instruction_file_bytes: 64 * 1024,
            instructions_total_bytes: 128 * 1024,
            instruction_ancestor_cap: 32,
        }
    }
}

/// How many omission records one set keeps. Not configurable, per spec section 4.
pub const MAX_OMISSION_RECORDS: usize = 32;

/// Where to look. A caller that wants no project instructions sets `discover` to false.
#[derive(Clone, Debug)]
pub struct InstructionConfig {
    /// Filenames to try in one directory, in order. The first that exists wins.
    pub filenames: Vec<String>,
    /// The user file directory. `None` resolves to `<home>/.config/rho`. When the home
    /// directory is also unavailable, rho reads no user file and records
    /// `OmissionReason::HomeUnavailable`. `None` never means "skip the user file".
    pub user_dir: Option<PathBuf>,
    /// The session root. rho walks from here up to the home directory. `None` records
    /// `OmissionReason::NoSessionRoot`, so a misconfigured caller is never mistaken for a
    /// repository with no instruction file.
    pub session_root: Option<PathBuf>,
    /// The directory that bounds the ancestor walk. `None` reads the environment. When
    /// that fails, rho walks zero ancestors and records `OmissionReason::HomeUnavailable`.
    /// An unknown boundary never widens into the ancestor cap. See spec section 2.
    pub home: Option<PathBuf>,
    /// False turns project discovery off. The user file still loads.
    pub discover: bool,
    pub limits: InstructionLimits,
}

impl InstructionConfig {
    /// The default configuration for a session root, reading `HOME` for the bounds.
    pub fn for_session_root(session_root: impl Into<PathBuf>) -> Self {
        Self {
            filenames: vec!["AGENTS.md".to_string()],
            user_dir: None,
            session_root: Some(session_root.into()),
            home: None,
            discover: true,
            limits: InstructionLimits::default(),
        }
    }
}

/// The path rho would read for the user file, given a home directory.
pub fn user_instruction_path(home: &Path, filename: &str) -> PathBuf {
    home.join(".config").join("rho").join(filename)
}

/// The user home directory, read from the environment.
///
/// The function reads `HOME`, and then `USERPROFILE` for a non-Unix host. This mirrors
/// `rho_skills::home_dir`, so the two features agree about where home is.
pub fn home_dir() -> Option<PathBuf> {
    for key in ["HOME", "USERPROFILE"] {
        if let Some(value) = std::env::var_os(key).filter(|v| !v.is_empty()) {
            return Some(PathBuf::from(value));
        }
    }
    None
}
