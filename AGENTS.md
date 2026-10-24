# fractic-server-error

The error type every Fractic Rust crate returns: `ServerError = Box<dyn
ServerErrorTrait>` carrying a message, optional debug detail, captured
context, a behaviour (how an API responds) and a tag. The
`define_server_error!` / `define_internal_error!` / `define_user_error!` /
`define_critical_error!` macros generate error structs with `::new(...)` and
`::with_debug(..., &cause)` constructors. Public, provided as-is
(`fractic-io/rust-server-error`, identity `fractic`).

## Read first

- `~/.agents/AGENTS.md` (workspace orientation; loaded automatically by Claude
  Code, read it if you are Codex or Grok) and `~/.agents/guides/rust.md`
  (style; §4 "Errors" is the consumer-side contract this crate defines).

## Layout

Flat library: `src/lib.rs` → `constants.rs`, `macros.rs` (the `define_*`
macros), `server_error.rs` (trait, behaviour, tag, context capture),
`standard_errors.rs` (`CriticalError`, `MultithreadingError`, `InitError`,
`NotImplementedError`, …).

## Working here

- No flake here: build and test through a product devshell, e.g.
  `nix develop /efs/fractic/storyengine/util --command cargo test`.
- Every crate depends on this one; a change to the trait or macro signatures
  is breaking for all of them (title it `Breaking: …`) and usually needs
  follow-up PRs in `fractic-cli-scaffolding` (`CliError` wraps these) and the
  AWS wrappers.
- New generic errors go to `standard_errors.rs` only when they are truly
  generic; domain errors belong in the crate that raises them.
- Commit / PR titles: imperative, trailing period.
