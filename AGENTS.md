Standards: archetype=library product=libffmpeg — rules via the `standards` MCP (https://standards.thmsn.dev). Repo-specific overrides below.

# AGENTS

- **[`README.md`](README.md) is the front door.** Change behaviour it describes (a command,
  the layout, what's generated, what's still a stub) and update it in the same commit. Ran a
  command from it that didn't work? Fix it before reporting back.
- **[`SHORTCUTS.md`](SHORTCUTS.md) before environment recon.** Anything about the *running
  system* rather than the source: check it first. After recon that cost more than a couple
  of commands, append the route you found (endpoint, command, mechanism), never the reading.

## Repo-specific overrides

- **`libffmpeg/tests/` exists** (the library rule is inline tests only). `cancellation.rs`
  and `span_fields.rs` each point the tools at a fake binary through the process-global
  `set_tool_path` override, so each needs its own test binary to avoid racing the inline
  tests. Everything else stays inline.
