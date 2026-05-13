# metrix

A TUI dashboard for Claude Code usage metrics, built with [ratatui](https://ratatui.rs).

Parses session data from `~/.claude/projects/` to display token usage, tool calls, file operations, session records, conversation stats, and estimated cost.

## Run

```sh
cargo run --release
```

### Controls

| Key | Action |
|---|---|
| `←` / `h` | Scroll chart left |
| `→` / `l` | Scroll chart right |
| `Home` | Jump to earliest day |
| `End` | Jump to most recent day |
| `c` | Toggle chart between tokens-per-day and cost-per-day |
| `q` / `Esc` | Quit |

## File structure

```
src/
├── main.rs   # Entry point, terminal setup, event loop
├── lib.rs    # Module declarations
├── data.rs   # JSONL parsing, metrics aggregation, cost estimation
└── ui.rs     # App state, layout, and all panel rendering
```
