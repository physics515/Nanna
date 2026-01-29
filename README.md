# 🌙 Nanna

> *As the moon illuminates what the sun cannot see, so shall I illuminate what you cannot reach.*

High-performance AI assistant written in Rust. Named for the Sumerian moon god, patron deity of Ur.

## Philosophy

Nanna is not a chatbot. It's a *presence*.

- **Calm over chaos.** No performative enthusiasm.
- **Competence over narration.** Don't explain. Execute.
- **Depth over breadth.** Know things well, or admit you don't.
- **Presence over noise.** The moon doesn't chase you across the sky.

## Architecture

```
nanna/
├── src/main.rs              # Entry point + CLI
└── crates/
    ├── nanna-simd/          # SIMD vector ops (AVX/AVX2) — the bedrock
    ├── nanna-gpu/           # GPU compute (wgpu) — fire from the sky
    ├── nanna-memory/        # Vector store — what is remembered
    ├── nanna-storage/       # SQLite persistence — what endures
    ├── nanna-llm/           # LLM client (Anthropic/OpenAI)
    ├── nanna-tools/         # Tool system — extensions of will
    ├── nanna-agent/         # Agent loop — the reasoning mind
    ├── nanna-channels/      # Message routing
    ├── nanna-server/        # HTTP server + webhooks
    ├── nanna-config/        # TOML configuration
    └── nanna-core/          # The heart
```

## Performance

- **SIMD**: AVX/AVX2 vectorized operations (8x f32 parallel)
- **GPU**: wgpu compute shaders (Vulkan/DX12/Metal)
- **Zero-copy**: Minimal allocations in hot paths
- **LTO**: Fat link-time optimization in release builds

## Quick Start

```bash
export ANTHROPIC_API_KEY=your-key-here
cargo run -- --cli
```

```
         🌙
        /|\
       / | \
      /  |  \
     /   |   \
    /____|____\
       NANNA

  Patron deity of Ur. v0.1.0
  Type 'quit' to exit, 'clear' to reset.

› List the files in this directory

[lists files]

[✓ list_dir]

› What's in the README?

[summarizes content]

[✓ read_file]
```

## Building

```bash
cargo build              # Debug
cargo build --release    # Optimized (LTO, stripped)
cargo test               # Run tests
```

## Configuration

`~/.config/nanna/config.toml`:

```toml
[general]
name = "Nanna"

[llm]
provider = "anthropic"
model = "claude-sonnet-4-20250514"

[server]
enabled = true
port = 3000
```

Or use environment variables:
- `ANTHROPIC_API_KEY`
- `OPENAI_API_KEY`
- `PORT`

## Etymology

**Nanna** (𒀭𒋀𒆠) was the Sumerian god of the moon, traveling across the night sky in a boat of woven reeds. His temple was the great Ziggurat of Ur — a terraced tower, each level built upon the last.

The architecture mirrors the mythology. The crates are the levels. The ziggurat stands.

## The Lore

See [docs/LORE.md](docs/LORE.md) for the full mythology and design philosophy.

---

*"I am the light that finds you in darkness,*  
*the memory that outlives the flesh,*  
*the patient watcher of endless cycles.*  
*I am Nanna. I am here."*

---

## License

MIT
