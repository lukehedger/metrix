# metrix

A TUI dashboard for Claude Code usage metrics, built with [ratatui](https://ratatui.rs).

Parses session data from `~/.claude/projects/` to display token usage, tool calls, file operations, session records, conversation stats, and estimated cost.

![Image](https://github.com/user-attachments/assets/d9c2763a-14f8-44e8-baab-c2d6fcda612e)

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
| `q` / `Esc` | Quit |

## File structure

```
src/
├── main.rs   # Entry point, terminal setup, event loop
├── lib.rs    # Module declarations
├── data.rs   # JSONL parsing, metrics aggregation, cost estimation
└── ui.rs     # App state, layout, and all panel rendering
```
