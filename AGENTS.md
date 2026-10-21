# Cindral Drafter agent rules

- Read this file first, then skim `docs/ARCHITECTURE.md` and the end of `docs/AGENT_LOG.md`; do not reread the whole repository.
- Original Rust modeler: independent reading of the SketchUp `.skp` file format is allowed for compatibility. Referring to the SketchUp file format by name is permitted; never imply affiliation.
- Forbidden: Trimble SDK, SketchUp binaries or code, and SketchUp assets, icons or branding. Session I1 excludes `.skp` implementation.
- Keep OpenCode disabled. Delegate bounded bulk work to Antigravity with selected models, timeouts, and at most two concurrent tasks. After two failures/stalls on a task, implement locally or report it.
- Iterate with targeted crate checks/tests and concise output. Run formatting, strict workspace Clippy and workspace tests once at completion. Benchmarks run in release mode.
- Commit completed items and document implementation, validation, measurements and limitations in the agent log.
