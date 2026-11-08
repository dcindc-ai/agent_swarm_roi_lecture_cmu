# rival-council

A swarm of reviewer agents, built on [AutoAgents](https://github.com/liquidos-ai/AutoAgents) (Rust), that argue about a lecture deck and get adjudicated. Every message in the swarm is logged so you can watch it assemble in the orbit monitor.

## What runs

Four agents on one pub/sub runtime (Ractor actors under the hood):

| Agent | Role |
|---|---|
| `policy_analyst` | fewer numbers, one question per slide, converted units |
| `instructional_designer` | scaffolding to the syllabus, three takeaways |
| `student` | what they'd write down, where they'd get lost |
| `adjudicator` | former CFO/CRO; rules on the debate |

Three rounds:

1. **Blind briefs.** The coordinator publishes the same material to the three rivals in parallel.
2. **Cross-talk.** When all three briefs are in, each rival receives the other two briefs and writes a rebuttal. This is the part the Claude.ai subagents could not do.
3. **Ruling.** The adjudicator receives all six documents and writes `out/ruling.md`.

The coordinator is not an LLM. It's the `drive()` loop in `src/main.rs` that watches runtime events (`TaskStarted`, `TaskComplete`, `TaskError`, tool calls) and opens the next round when a round completes. That's the architecture-and-guidelines part; the agents do the rest.

## Run it (in Claude Code, on your machine)

```bash
# needs a Rust toolchain: https://rustup.rs
export ANTHROPIC_API_KEY=sk-ant-...
export ANTHROPIC_MODEL=claude-sonnet-5-5        # optional; this is the default

cargo run --release                             # uses input/material.md
cargo run --release -- path/to/other-material.md
```

Outputs land in `out/`: one brief and one rebuttal per rival, `ruling.md`, and `runs.jsonl` (the event log). First build takes a few minutes; after that it's seconds.

To use an OpenAI-compatible endpoint instead (including a local one), set `OPENAI_API_KEY`, and optionally `OPENAI_BASE_URL` and `OPENAI_MODEL`.

## Dry run with no API key

`stub-model.py` is a stand-in OpenAI-compatible model that answers in each agent's role after a short delay. Use it to watch the swarm mechanics (rounds, cross-talk, logging, the monitor) without spending tokens:

```bash
python3 stub-model.py &                      # listens on 127.0.0.1:8790
OPENAI_API_KEY=stub OPENAI_BASE_URL=http://127.0.0.1:8790/v1 OPENAI_MODEL=stub cargo run --release
```

## Watch it live

In a second terminal:

```bash
./serve-monitor.sh          # serves monitor/ and out/ on http://localhost:8787
```

Open http://localhost:8787/orbits-monitor.html, paste `http://localhost:8787/runs.jsonl` into the **Live source** box, and click **Connect**. Then start `cargo run`. Goals (round1, round2, round3) sit in the center; agents orbit; each message flashes along its edge. Red dashed edges are one rival's brief going to another rival, which is the cross-talk.

The event schema is five fields, so any other agent runtime can feed the same page by appending lines:

```json
{"ts":"2026-11-08T12:01:00","from":"student","to":"policy_analyst","kind":"read","goal":"round2","label":"student brief sent to policy_analyst"}
```

`kind` is one of `task`, `result`, `write`, `read`, `post`.

## Change the swarm

- **New rival:** add an `#[agent(name, description)]` struct, add its name to `RIVALS`, and register it with `AgentBuilder` in `main()`. Rounds and logging pick it up automatically.
- **More rounds:** add a `round3` map to `State` and another branch in `drive()`.
- **Different material:** any markdown file as the first argument.
- **Tools:** AutoAgents agents can carry tools (see the repo's `examples/basic`); `ToolCallRequested` events already log to the monitor.

## Known limits

- AutoAgents is at 0.4.0 and pinned here to the GitHub main branch; if the API moves, pin a commit with `rev = "..."` in `Cargo.toml`.
- No retries on a model error: the failing agent's error is logged and the round stalls. Ctrl-C shuts the environment down cleanly.
- The rebuttal round sends the full briefs as prompt text; with long material, watch token cost (three briefs x three rivals, then the adjudicator reads all six).
