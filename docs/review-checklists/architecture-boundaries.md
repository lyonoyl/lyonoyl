# Architecture boundary review

Applies when adding modules, ports, adapters, or commands, when changing the directory structure, and whenever an abstraction seems to leak.

Read first: [the working contract](../../AGENTS.md), section 1—the directory map and what is forbidden where.

The guiding question: **will this survive three years, and where will the abstraction leak?**

## Checklist

1. **Dependency direction.** Run `scripts/check-layering.sh` and read its output. Also check: `crates/lyonoyl-domain/Cargo.toml` depends on `thiserror` and nothing else, and `application/` holds no `use crate::infrastructure`, `use rusqlite`, or `use tauri`.
2. **Port signatures.** Do they contain adapter types—`rusqlite::Error`, `async_imap::*`, `reqwest::*`? Is the sync cursor typed as `u64` instead of an opaque `SyncCursor`? Is there a `Uid` where a `MessageId` belongs, or a folder path where a `FolderId` belongs?
3. **Trait size.** Is a port drifting toward "25 methods, half returning `Unsupported`"? Are independently varying conversations separated, as the seven mail ports in [the architecture overview](../architecture/overview.md) are?
4. **Logic in commands.** Does `commands/` contain any domain branching, SQL, or business rule? All of that belongs to a use case.
5. **Anemic domain.** Has `crates/lyonoyl-domain/` become a pile of `pub struct`s with public fields and no rules? The real logic—`deletions_for`, `after_uid_validity_changed`, `effective_flags`—lives there.
6. **Read model.** Does list reading go through domain objects (needless materialization on the hot path)? Has a write appeared in `read_model/`?
7. **Shared core.** Has a new slice invented its own message identity, its own cursor, its own error format, or its own way of calling `invoke`? That is how vertical slices turn into spaghetti.
8. **Reversibility.** For each new decision ask: if a different adapter is needed in six months, is that adding a file or rewriting? If rewriting, the decision is one to settle **before** the code is written.

## Reporting

Cite file and line for every finding, most severe first. For each, give a concrete failure scenario—inputs and the resulting behavior—and the minimal fix. If nothing is found, say so; do not pad the list.
