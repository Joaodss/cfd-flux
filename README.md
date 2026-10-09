# live-fluids

Simulador de dinâmica de fluidos computacional (CFD) no browser. Desenha-se o domínio **pixel a pixel**, definem-se paredes, entradas e saídas de fluido, temperaturas e propriedades das fronteiras, e a simulação corre num servidor local em Rust com GPU (CUDA ou wgpu), com CPU como fallback. Os resultados são enviados em streaming e visualizados no browser.

> Estado: **Fase 0 (fundações)** — formato de cena, validação, formato de frames, CLI e esqueleto web. Ainda sem solver.

## Ideia em 30 segundos

```
 Browser (editor pixel a pixel)  ──HTTP/JSON+binário──▶  cfd-server (Rust/axum)
        ▲                                                   │
        │ WebSocket (frames de resultados)                  ▼
        └──────────────────────────────────────────  Jobs ──▶ Solver
                                                              ├─ CUDA (NVIDIA)
                                                              ├─ wgpu / WGSL (qualquer GPU)
                                                              └─ CPU (rayon) — referência numérica
```

- **Gases e líquidos** no MVP (2D); depois granular, plasma e mudanças de estado; **3D** numa segunda etapa.
- **Primeiro método:** Lattice Boltzmann (D2Q9 + D2Q5 térmico, superfície livre) — encaixa numa grelha de pixels e é muito eficiente em GPU.
- **Validação quantitativa** contra casos publicados (Ghia, Schäfer-Turek, de Vahl Davis, Martin-Moyce).

## Começar

```bash
cargo test --workspace                          # testes Rust
cargo run -p cfd-cli -- example --list          # cenas de exemplo
cargo run -p cfd-cli -- validate scenes/*.json  # validar cenas
cd apps/web && npm install && npm run dev       # frontend
```

Requisitos e problemas conhecidos (Windows): [docs/desenvolvimento.md](docs/desenvolvimento.md).

## Estrutura

```
crates/
  cfd-core/    formato Scene, codificação das camadas, validação, exemplos
  cfd-io/      formato binário dos frames de resultados
  cfd-cli/     CLI: schema, example, validate (mais tarde run, bench, compare)
  cfd-lbm/     solver LBM em CPU            (Fase 1)
  cfd-gpu/     backend wgpu/WGSL            (Fase 2a)
  cfd-cuda/    backend CUDA, feature `cuda` (Fase 2b)
  cfd-server/  API HTTP/WebSocket e jobs    (Fase 3)
apps/web/      frontend React + TypeScript + Vite
schema/        JSON Schema da cena (gerado a partir do Rust)
scenes/        cenas de exemplo (geradas)
sandbox/       protótipos de aprendizagem (fora do workspace)
docs/          documentação de planeamento (PT)
```

## Documentação

| Documento | Conteúdo |
|-----------|----------|
| [01 — Visão e requisitos](docs/01-visao-e-requisitos.md) | Objetivos, âmbito, utilizadores, requisitos funcionais e não funcionais |
| [02 — Arquitetura](docs/02-arquitetura.md) | Componentes, fluxo de um pedido, stack tecnológica, estrutura do repositório |
| [03 — Modelo de dados](docs/03-modelo-de-dados.md) | Formato da cena, camadas de pixels, tipos de fronteira, materiais, formato dos resultados, API |
| [04 — Métodos numéricos](docs/04-metodos-numericos.md) | LBM, térmico, projeção, líquidos, compressível, estabilidade, casos de validação |
| [05 — Plano de implementação](docs/05-plano-implementacao.md) | Fases, tarefas, critérios de aceitação, MVP |
| [06 — Otimizações e IA](docs/06-otimizacoes-e-ia.md) | AMR, multigrid, multi-GPU, malhas modernas, modelos substitutos com IA |
| [07 — Decisões e perguntas](docs/07-decisoes-e-perguntas.md) | ADRs e questões por decidir |
| [08 — Guia de validação](docs/08-guia-validacao.md) | Como medir a precisão: referências, erros, ordem de convergência |
| [Desenvolvimento](docs/desenvolvimento.md) | Ambiente, comandos, convenções |
| [Sessões](docs/sessoes/) | Checklists das sessões de trabalho |

## Licença

Licenciado à escolha de quem usa, sob:

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

Salvo indicação explícita em contrário, qualquer contribuição submetida para inclusão neste projeto, tal como definido na licença Apache-2.0, é licenciada como acima, sem termos ou condições adicionais.
