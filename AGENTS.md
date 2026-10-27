# fractic-server-error

The error type every Fractic Rust crate returns: `ServerError = Box<dyn
ServerErrorTrait>` carrying a message, optional debug detail, captured
context, a behaviour (how an API responds) and a tag. The
`define_server_error!` / `define_internal_error!` / `define_user_error!` /
`define_critical_error!` macros generate error structs with `::new(...)` and
`::with_debug(..., &cause)` constructors. Consumer side:
`/efs/fractic/_/rust.md` §4 "Errors". Flat library: `src/lib.rs` →
`constants.rs`, `macros.rs`, `server_error.rs` (trait, behaviour, tag,
context capture), `standard_errors.rs` (`CriticalError`,
`MultithreadingError`, `InitError`, `NotImplementedError`, …).

- A change to the trait or macro signatures is breaking for every crate
  (`Breaking: …`) and usually needs follow-up PRs in
  `fractic-cli-scaffolding` (`CliError` wraps these) and the AWS wrappers.
- New entries in `standard_errors.rs` only when truly generic; domain errors
  belong in the crate that raises them.
- Commit / PR titles: imperative, trailing period.
