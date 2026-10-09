# 02 — Arquitetura

## 1. Visão geral

```
┌──────────────────────────── Browser ─────────────────────────────┐
│  Editor (Canvas2D)   Painel de propriedades   Visualizador (WebGL2/WebGPU) │
│        │                      │                         ▲                 │
│        └──── Scene (estado TS) ┘                         │ frames binários │
└───────────────┬──────────────────────────────────────────┼─────────────────┘
                │ POST /api/simulations (JSON + camadas)   │ WS /api/simulations/{id}/stream
                ▼                                          │
┌──────────────────────────── cfd-server (Rust, axum) ─────┴─────────────────┐
│  Validação ─▶ Job Manager ─▶ Fila (in-process → Redis/NATS)                │
│  Metadados (SQLite → Postgres)   Resultados (disco → S3/MinIO)            │
└───────────────────────────────┬────────────────────────────────────────────┘
                                ▼
┌──────────────────────────── cfd-worker (Rust) ────────────────────────────┐
│  Scene → Domain (grelha, máscaras, materiais, unidades de rede)           │
│  trait Solver ──┬── LbmSolver ──┬── backend GPU (wgpu + WGSL)             │
│                 │               ├── backend GPU (CUDA via cudarc)         │
│                 │               └── backend CPU (rayon + SIMD)            │
│                 ├── ProjectionSolver (Fase 7)                             │
│                 └── ... (FLIP, FV compressível, surrogate IA)              │
│  Output: amostragem de campos → codificação (f16 + zstd) → stream/disco   │
└───────────────────────────────────────────────────────────────────────────┘
```

No início **servidor e worker correm no mesmo processo** (tarefas tokio + thread dedicada ao solver). A separação em processos/máquinas só acontece quando houver necessidade (várias GPUs, vários utilizadores).

## 2. Componentes

### 2.1 Frontend — `apps/web`
- **Stack:** TypeScript + Vite + React, Zustand, Radix/shadcn + Tailwind, uPlot; Vitest + Playwright; Three.js/react-three-fiber na Etapa 3D (ver [ADR-005](07-decisoes-e-perguntas.md)).
- **Editor:** Canvas2D com camadas guardadas como `Uint8Array`/`Uint16Array` (uma por camada lógica). Zoom/pan, grelha visível a partir de certo zoom, desenho com algoritmos de Bresenham / scanline fill.
- **Estado:** store simples (Zustand ou equivalente) com histórico para undo/redo (diffs por retângulo afetado).
- **Visualizador:** WebGL2 (fallback universal) com shaders para colormaps; WebGPU quando disponível para partículas e linhas de corrente.
- **Comunicação:** REST para CRUD, WebSocket binário para frames.
- **Opcional futuro:** o núcleo do solver compilado para WASM/WebGPU para pré-visualização local de baixa resolução.

### 2.2 API — `crates/cfd-server`
- **Stack:** Rust, `axum`, `tokio`, `serde`, `tower-http` (CORS, compressão, limites), `tracing`.
- **Responsabilidades:** autenticação (mais tarde), validação do pedido, gestão de jobs, streaming, servir resultados.
- **Persistência:** `sqlx` com SQLite no início, Postgres em produção. Resultados em ficheiros (abstração `ResultStore` → disco local / S3).

### 2.3 Fila e workers
- **MVP:** fila em memória (`tokio::sync::mpsc`) + semáforo por dispositivo (1 job por GPU de cada vez, N jobs CPU).
- **Escala:** Redis Streams ou NATS JetStream; workers stateless registam as suas capacidades (GPU, memória, backends suportados).
- **Agendamento:** escolha do backend segundo memória necessária estimada (`células × bytes por célula`) e disponibilidade.

### 2.4 Núcleo de simulação — `crates/cfd-core` e solvers
- `cfd-core`: tipos comuns (Scene, Domain, Field, unidades, conversões físico↔rede, materiais, trait `Solver`, trait `Backend`).
- `cfd-lbm`: LBM D2Q9 (+ D2Q5 térmico), kernels CPU.
- `cfd-gpu`: inicialização de `wgpu`, gestão de buffers, pipelines de compute, shaders WGSL.
- Interface comum:

```rust
pub trait Solver {
    fn init(domain: &Domain, cfg: &SolverConfig, backend: BackendKind) -> Result<Self> where Self: Sized;
    fn step(&mut self, n: u32) -> Result<()>;          // avança n passos
    fn sample(&mut self, req: &SampleRequest) -> Result<FieldSet>; // copia campos pedidos para CPU
    fn update_boundaries(&mut self, patch: &BoundaryPatch) -> Result<()>; // modo interativo
    fn diagnostics(&self) -> Diagnostics;              // massa total, energia, max vel, NaN check
}
```

### 2.5 CLI — `crates/cfd-cli`
Executa cenas a partir de ficheiros sem servidor: essencial para desenvolvimento, benchmarks e testes de validação (`cfd run scene.json --backend gpu --out out/`).

## 3. Fluxo de um pedido

1. Utilizador desenha e configura → frontend serializa a `Scene` (ver [03](03-modelo-de-dados.md)).
2. `POST /api/simulations` → servidor valida esquema, limites e consistência física; estima memória e custo.
3. Job criado (`queued`) → devolve `{ id }`.
4. Cliente abre `WS /api/simulations/{id}/stream`.
5. Worker: `Scene → Domain` (rasterização, unidades de rede, verificação de estabilidade) → `Solver::init` → ciclo `step` / `sample` / `encode` / `publish`.
6. Cada frame é enviado pelo WebSocket e escrito no `ResultStore`.
7. Fim (`completed` / `failed` / `cancelled`) → resumo com diagnósticos.

## 4. Stack tecnológica (proposta)

| Camada | Escolha | Alternativas consideradas |
|--------|---------|---------------------------|
| Linguagem servidor/solver | **Rust** | C++ (mais bibliotecas CFD, menos segurança), Julia (ótimo para prototipar, deploy mais difícil) |
| GPU | **wgpu + WGSL** (Vulkan/DX12/Metal) **e CUDA via `cudarc`** (NVIDIA, feature `cuda`) — ambos desde a Fase 2 | rust-gpu, HIP (AMD) |
| CPU paralelo | **rayon** + `std::simd`/`wide` | OpenMP via C++ |
| HTTP/WS | **axum** | actix-web |
| Serialização | **serde** (JSON) + binário próprio / MessagePack | Protobuf/FlatBuffers |
| Compressão | **zstd** | lz4 |
| BD | SQLite → Postgres (`sqlx`) | — |
| Frontend | **TypeScript + Vite + React** | Svelte, SolidJS |
| Render resultados | WebGL2 → WebGPU | Canvas2D (lento) |
| IA (inferência) | `ort` (ONNX Runtime) ou `burn`/`candle` | Serviço Python separado |
| IA (treino) | Python + PyTorch (offline) | JAX |

## 5. Estrutura do repositório (monorepo)

```
CFDWebSimulator/
├── README.md, CLAUDE.md, docs/
├── Cargo.toml                 # workspace Rust
├── crates/
│   ├── cfd-core/              # tipos, unidades, Scene→Domain, traits
│   ├── cfd-lbm/               # solver LBM (CPU) + lógica comum
│   ├── cfd-gpu/               # infraestrutura wgpu + shaders WGSL
│   ├── cfd-cuda/              # backend CUDA (cudarc + kernels .cu), feature `cuda`
│   ├── cfd-io/                # formatos de cena e de resultados, codificação de frames
│   ├── cfd-server/            # API axum + jobs + streaming
│   └── cfd-cli/               # CLI de execução/benchmark
├── apps/
│   └── web/                   # frontend TS
├── schema/                    # JSON Schema da Scene (fonte de verdade partilhada)
├── scenes/                    # cenas de exemplo e de validação
├── validation/                # dados de referência (Ghia, Schäfer-Turek, ...) e scripts
└── tools/                     # scripts (benchmarks, geração de dados para IA)
```

**Formato da cena:** os tipos Rust em `cfd-core::scene` (`serde` + `schemars`) são a fonte de verdade. O JSON Schema em `schema/scene.schema.json` é gerado a partir deles (`cfd-cli schema`) e guardado no repositório; um teste falha se estiver desatualizado. Os tipos TS do frontend são gerados a partir do schema (`npm run gen:types`). Assim: Rust → JSON Schema → TypeScript, sem duplicação manual.

Protótipos de aprendizagem (código descartável) ficam em `sandbox/`, fora do workspace Cargo.

## 6. Segurança e limites

- Limites rígidos: tamanho do domínio, número de passos, tempo de parede, frames armazenados, jobs simultâneos por utilizador.
- Validação de todos os índices/IDs (sem pânico no worker por input malicioso); timeouts por job.
- Nenhum código do utilizador é executado (expressões para perfis de entrada, se forem suportadas, usam um avaliador restrito e sandboxed).
- Rate limiting na API; autenticação (sessões ou OAuth) antes de abrir ao público.

## 7. Deploy (futuro)

- Container com runtime Vulkan (NVIDIA: `nvidia-container-toolkit`).
- Um processo `cfd-server` + N `cfd-worker` por máquina com GPU.
- Observabilidade: `tracing` + OpenTelemetry → Prometheus/Grafana.
