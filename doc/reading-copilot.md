# Reading Copilot

Ink Reader treats AI as a reading agent, not as a generic chat pane. The first
version deliberately has a small authority surface: it can read the current
terminal page, follow a task prompt, and return streamed text. It has no tools
and cannot change the book, filesystem, bookmarks, or network destination.

## Interaction design

Press `c` while reading. Nothing is sent until an action is selected:

- `e` explains the thesis, terms, assumptions, and argument flow.
- `t` translates into Simplified Chinese while preserving formulas and citations.
- `s` produces a compact study summary and highlights a comprehension trap.
- `r` reconstructs mathematical or logical reasoning step by step.
- `a` accepts a free-form question. From an answer, it starts a follow-up.

The response streams into a right-side panel so the source page remains visible
while the reader compares explanation and evidence. The panel is 40–64 columns
wide on terminals at least 90 columns wide; smaller terminals fall back to a
full-screen view rather than squeezing both panes into unreadable columns.
Opening and closing it reflows the page to the actual reader width and preserves
the approximate chapter position.

`x` cancels work and `j`/`k` scroll. Thinking tokens are not exposed as an
answer: the UI shows a `Reasoning…` state and renders only the model's final
response. The endpoint and `LOCAL`/`REMOTE` privacy state remain visible.

The context is the visible reflowed page, book title, and current chapter or
section title. This is intentionally smaller than a whole chapter: it reduces
latency, keeps the claim boundary obvious, and caps local memory use. The agent
is explicitly told to distinguish excerpt-supported claims from outside
knowledge.

## Why Rig

The implementation uses `rig-core` rather than a hand-written Ollama client.
Rig supplies the Agent, streaming, provider, tool, memory, and vector-store
boundaries needed for an Agent First reader. The current agent is page-scoped
and tool-free, but this avoids a transport rewrite when later versions add:

1. read-only book tools such as `current_page`, `chapter_outline`, and
   `nearby_sections`;
2. embeddings and cross-chapter retrieval with citations back to book positions;
3. per-book conversation memory and generated reading notes;
4. other Rig providers selected by explicit configuration.

`copilot.rs` owns provider/agent construction and background streaming;
`app.rs` owns interaction state and exact visible-page context; `ui/copilot.rs`
only renders. Provider work never blocks the ratatui event loop.

## Model decision

The default is `qwen3.5:4b`, currently about a 3.4 GB Q4 model in Ollama. It is
the best starting point here because one relatively small model covers Chinese
and English, translation, general explanation, mathematical reasoning, and
future image input. Keeping one model resident also avoids the load/unload delay
of routing every task between two large local models.

`phi4-mini-reasoning` remains a useful optional reasoning model. It is a 3.8B,
roughly 3.2 GB Q4 model aimed at multi-step mathematical reasoning. Configure it
only if the extra model switch and disk/RAM cost are acceptable:

```bash
--copilot-reasoning-model phi4-mini-reasoning
```

`qwen2.5:7b` is still usable but is no longer the default: it is larger and an
older generation than the selected Qwen model. Model quality is workload- and
quantization-dependent, so these defaults are operational choices rather than a
claim that a 4B model can replace a large hosted model for every paper.

The agent requests an 8K context even when a model advertises much more. A
larger context increases KV-cache memory and does not benefit a page-scoped
request. Fast tasks disable thinking; only deep analysis enables it. Models are
kept alive for five minutes to make consecutive questions responsive.

Official model pages:

- <https://ollama.com/library/qwen3.5:4b>
- <https://ollama.com/library/phi4-mini-reasoning>

## Setup without making Ollama a build dependency

Ollama is optional and is not installed or started by Ink Reader itself. If it
is missing, ordinary reading continues to work and Copilot shows an actionable
error.

### WSL2 and desktop Linux

Install Ollama using the current instructions at <https://ollama.com/download/linux>.
Then start its service and pull the default model:

```bash
ollama pull qwen3.5:4b
ollama serve  # only when it is not already running as a service
```

The official archive can also be kept entirely under the current user without
sudo while preserving its `bin/` and `lib/ollama/` layout:

```bash
mkdir -p "$HOME/.local"
tar --zstd -xf ollama-linux-amd64.tar.zst -C "$HOME/.local"
export PATH="$HOME/.local/bin:$PATH"
ollama serve
```

For a long-lived WSL2/Linux installation with systemd enabled, install the
included user service instead of starting a disposable shell process:

```bash
install -Dm644 contrib/systemd/user/ollama.service \
  "$HOME/.config/systemd/user/ollama.service"
systemctl --user daemon-reload
systemctl --user enable --now ollama.service
loginctl enable-linger "$USER"
```

The service runs `~/.local/bin/ollama`, restarts after failures, and listens only
on `127.0.0.1:11434`. Optional environment overrides can be placed in
`~/.config/ollama/environment`. Use `systemctl --user status ollama` and
`journalctl --user -u ollama -f` for status and logs.

For WSL2, installing inside WSL is the least surprising network layout. NVIDIA
GPU passthrough can accelerate supported hardware; CPU inference still works but
will be slower. A Windows-native Ollama server can also be used, but its address
depends on the WSL networking mode. Verify the exact address first:

```bash
curl http://127.0.0.1:11434/api/tags
```

If that fails, configure `--ollama-url` to the explicitly exposed Windows host
address. Do not bind Ollama broadly to the LAN without access controls.

### macOS

Install the official Ollama app from <https://ollama.com/download/mac>, launch
it, and run the same `ollama pull qwen3.5:4b` command. Ollama uses Metal on
supported Apple GPUs.

## Local, remote, and cloud endpoints

The default is `http://127.0.0.1:11434`, which keeps excerpts local. A remote
Ollama-compatible endpoint is explicit:

```bash
ink-reader paper.pdf --ollama-url https://llm.example.net \
  --copilot-model qwen3.5:4b
```

Set `INK_READER_OLLAMA_API_KEY` or `OLLAMA_API_KEY` for a bearer-authenticated
endpoint. Ollama Cloud models may also be used through a signed-in local Ollama
server, preserving the same local API address. In both cases the overlay marks
the endpoint as remote when the configured address itself is non-loopback.

API keys are read from environment variables and are never displayed. They
should not be passed in endpoint URLs or committed to project files.

## Current limits

- Context is current-page text, not a whole-book RAG index.
- Image-only pages and covers are rejected; multimodal page analysis is future work.
- A follow-up includes the immediately preceding task and answer, but persistent
  multi-turn/per-book conversation memory is not enabled yet.
- Model speed depends heavily on whether Ollama detects a GPU. The same model
  can be interactive on GPU and substantially slower under CPU-only WSL2.
