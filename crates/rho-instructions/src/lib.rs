//! Project instruction discovery and prompt rendering for rho.
//!
//! rho reads `AGENTS.md` and renders it into the stable prompt prefix. See
//! `docs/specs/20260821-131725-SPEC-project-instructions.md`.
//!
//! A project file comes from the repository under edit, so it is untrusted input. rho
//! reads it and grants it nothing. See D-project-instructions-are-authority-inert.

mod discover;
mod prompt;
mod types;

pub use discover::gather;
pub use prompt::prompt_block;
pub use types::{
    Instruction, InstructionConfig, InstructionLimits, InstructionOrigin, InstructionSet,
    MAX_OMISSION_RECORDS, Omission, OmissionReason, home_dir, user_instruction_path,
};
