# Ambiente de desenvolvimento

## Requisitos

| Ferramenta | Versão | Para quê |
|------------|--------|----------|
| Rust (rustup) | ≥ 1.85 (testado com 1.90) | Workspace `crates/` |
| Node.js | ≥ 22 (testado com 24) | `apps/web` |
| CUDA Toolkit | 12.x *(só a partir da Fase 2b)* | Backend `cfd-cuda` (NVRTC) |

### Windows: toolchain Rust

Recomendado: toolchain **MSVC** (`stable-x86_64-pc-windows-msvc`) com o **Visual Studio Build Tools 2022**
e a carga de trabalho "Desenvolvimento para desktop com C++". É o toolchain padrão do Rust em Windows e
o compilador de C++ do MSVC também é exigido pelo CUDA em Windows.

```powershell
winget install Microsoft.VisualStudio.2022.BuildTools --override "--quiet --wait --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"
rustup default stable-x86_64-pc-windows-msvc
```

> **Problema conhecido (2026-10-09):** com o toolchain `windows-gnu` e o MinGW.org antigo em `C:\MinGW\bin`
> no PATH, `cargo build` falha em `windows-sys` ("Dlltool could not create import library"): falta um
> assembler x86-64. `cargo check`/`clippy` funcionam; para compilar/testar usar o toolchain MSVC (acima)
> ou o WSL.

### Alternativa: WSL (Ubuntu)

```bash
cd /mnt/c/Users/<user>/Desktop/GithubProjects/live-fluids
CARGO_TARGET_DIR=$HOME/.cache/live-fluids-target cargo test --workspace
```
(`CARGO_TARGET_DIR` fora de `/mnt/c` evita misturar binários Windows/Linux e é muito mais rápido.)

## Comandos

```bash
# Rust
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace

# Formato da cena: depois de alterar crates/cfd-core/src/scene.rs ou examples.rs
cargo run -p cfd-cli -- schema schema/scene.schema.json
cargo run -p cfd-cli -- example --all scenes
cd apps/web && npm run gen:types

# Validar cenas
cargo run -p cfd-cli -- validate scenes/*.json

# Web (em apps/web)
npm install
npm run dev          # servidor de desenvolvimento
npm test             # Vitest (valida scenes/ contra o JSON Schema)
npm run typecheck && npm run lint && npm run format:check && npm run build
```

## Convenções

- Código, identificadores, comentários e mensagens de commit em **inglês** (projeto open source);
  documentação de planeamento em `docs/` em português.
- `schema/scene.schema.json`, `scenes/*.json` e `apps/web/src/generated/` são **gerados** — não editar à mão.
  Testes (Rust) e o CI (web) falham se estiverem desatualizados.
