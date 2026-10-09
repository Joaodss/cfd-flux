# 05 — Plano de implementação

## 0. Estratégia

1. **Solver primeiro, sem web.** Um solver validado e rápido é o núcleo; a UI é inútil sem ele. A CLI permite iterar sem servidor nem browser.
2. **CPU como referência, GPU como produção.** O backend CPU é simples e correto; o GPU é comparado contra ele em cada teste.
3. **Fatias verticais cedo.** Assim que houver um solver CPU mínimo, ligá-lo a uma API e a um canvas tosco (Fase 3/4 podem começar em paralelo com a Fase 2).
4. **Contratos estáveis:** o formato da `Scene` e o protocolo de frames são definidos na Fase 0 e evoluem com versões.

Esforço indicativo por tarefa: **P** (≤ 1 dia), **M** (2–4 dias), **G** (1–2 semanas). Valores para uma pessoa a tempo parcial devem ser multiplicados conforme a disponibilidade.

```
Fase 0 Fundações ─▶ Fase 1 Solver CPU ─┬─▶ Fase 2 GPU ───────────────┐
                                       ├─▶ Fase 3 Servidor ──────────┼─▶ MVP ─▶ Fases 6–10
                                       └─▶ Fase 4 Editor ─▶ Fase 5 Vis┘
                                       (MVP = Fases 0–5 + 6a: líquidos com superfície livre)
```

---

## Fase 0 — Fundações

**Objetivo:** repositório, ferramentas e contratos prontos.

- [x] P — `git init`, `.gitignore`, licença, estrutura de pastas ([02 §5](02-arquitetura.md#5-estrutura-do-repositório-monorepo)).
- [x] P — Workspace Cargo com crates vazios (`cfd-core`, `cfd-lbm`, `cfd-gpu`, `cfd-cuda`, `cfd-io`, `cfd-server`, `cfd-cli`); `rustfmt`, `clippy` com `-D warnings`.
- [x] P — Projeto `apps/web` (Vite + React + TS), oxlint, Prettier, Vitest.
- [x] M — Formato `Scene` v1: tipos Rust (`serde` + `schemars`) → JSON Schema gerado em `schema/` (teste falha se estiver desatualizado) → tipos TS gerados (`json-schema-to-typescript`); exemplos validados nos dois lados; testes de ida-e-volta.
- [x] P — Validação semântica da cena em Rust (tamanhos das camadas, IDs, tipos de célula ↔ elementos, adjacência de inlets/outlets).
- [x] P — Licença MIT OR Apache-2.0 (`LICENSE-MIT`, `LICENSE-APACHE`).
- [x] P — Especificação do formato binário de frames (`cfd-io`), com teste de codificação/descodificação.
- [x] P — CI (GitHub Actions): build, testes, lint em Rust e TS (Windows + Linux); build sem a feature `cuda` no CI público. *(workflow criado; confirmar no primeiro push)*
- [ ] P — Instalar e verificar o CUDA Toolkit local (`nvcc --version`, `nvidia-smi`) e documentar a versão mínima em `docs/`. *(adiado: só é preciso na Fase 2b; ver [desenvolvimento.md](desenvolvimento.md))*
- [x] P — 4 cenas de exemplo em `scenes/` (canal, cilindro Re 100, cavidade aquecida Ra 1e5, dam break), geradas por `cfd-cli example --all scenes`.

### Protótipos de aprendizagem (Rust + GPU) — antes ou em paralelo com o resto da Fase 0

Objetivo: ganhar fluência em Rust e GPU com código descartável, antes de escrever código "a sério". Ficam em `sandbox/` (fora do workspace principal).

- [ ] P — Rustlings ou os capítulos 1–10 do *Rust Book*; atenção a ownership/borrowing, `Result`/`?`, traits (≈ interfaces de C#), `enum` com dados (≈ discriminated unions).
- [ ] M — LBM D2Q9 mínimo num único ficheiro `main.rs` (cavidade 128², escreve PNG com a crate `image`). Primeiro sequencial, depois com `rayon` (`par_chunks_mut`).
- [ ] P — "Hello compute" em `wgpu`: somar dois vetores num compute shader WGSL e ler o resultado.
- [ ] P — "Hello compute" em `cudarc`: o mesmo com um kernel `.cu` compilado por NVRTC.
- [ ] P — Portar o kernel do LBM mínimo para wgpu **ou** CUDA e comparar MLUPS com a versão CPU.

Notas para quem vem de C#:
- Em vez de grafos de objetos com referências cruzadas (que lutam com o borrow checker), usar **arrays + índices** (é exatamente o que o solver precisa).
- Erros: `thiserror` nas bibliotecas, `anyhow` nas aplicações (CLI/servidor).
- `async` em Rust (tokio) ≈ `async/await` de C#, mas o solver corre numa thread normal, fora do runtime async.
- Ferramentas: VS Code + rust-analyzer (ou RustRover), `cargo clippy`, `cargo nextest`, `criterion` para benchmarks, Nsight Compute para perfilar kernels CUDA, RenderDoc/Nsight Graphics para wgpu.

**Aceitação:** `cargo test` e `npm test` verdes no CI; uma cena de exemplo é validada pelo schema nos dois lados.

---

## Fase 1 — Núcleo do solver em CPU (LBM)

**Objetivo:** LBM 2D correto e validado, executável pela CLI.

- [ ] M — `cfd-core`: `Domain`, flags por célula, conversão `Scene → Domain`, módulo de unidades (físico ↔ rede) com verificação de estabilidade e avisos.
- [ ] M — `cfd-lbm`: D2Q9 BGK, kernel fundido stream-collide (pull), ping-pong, paralelizado com `rayon` por linhas.
- [ ] M — Condições de fronteira: bounce-back, parede móvel, inlet de velocidade (Zou-He), outlet de pressão e zero-gradient, periódica.
- [ ] P — Forçamento de Guo (gravidade).
- [ ] M — Operador TRT (default) mantendo BGK como opção.
- [ ] M — Térmico D2Q5 + Boussinesq; BCs térmicas fixa/adiabática/fluxo.
- [ ] P — Diagnósticos: massa, energia, velocidade máxima, deteção NaN.
- [ ] M — `cfd-cli run`: lê cena, executa, escreve PNGs (colormap) e frames binários; `cfd-cli bench` reporta MLUPS.
- [ ] G — Suite de validação automática (`cfd-cli validate`, metodologia em [08](08-guia-validacao.md)), incluindo testes de ordem de convergência: Poiseuille, Couette, Taylor-Green, cavidade (Ghia), cilindro (Schäfer-Turek), cavidade aquecida (de Vahl Davis). Gera relatório com erros e gráficos.
- [ ] P — Cálculo de forças em sólidos (momentum exchange) para C_D/C_L; séries temporais das sondas.

**Aceitação:** todos os casos de [04 §7](04-metodos-numericos.md#7-validação) aplicáveis dentro das tolerâncias; ≥ 50 MLUPS em 8 núcleos; esteira de von Kármán visível no cilindro Re 100.

---

## Fase 2 — Backends GPU (wgpu + CUDA)

**Objetivo:** mesmo solver em GPU, 10–50× mais rápido, com fallback automático, em **duas engines comparáveis**: wgpu/WGSL (portável) e CUDA (NVIDIA). Ver [ADR-002](07-decisoes-e-perguntas.md).

Ordem sugerida: wgpu primeiro (2a), CUDA logo a seguir (2b), comparação no fim (2c). Os kernels partilham a mesma disposição de memória (SoA, mesma indexação, mesmas flags) para que a comparação seja justa e os testes reutilizáveis.

### 2a — wgpu / WGSL

- [ ] M — `cfd-gpu`: deteção de adaptadores, escolha de dispositivo, limites (`max_storage_buffer_binding_size`), fallback para CPU se não houver GPU/memória.
- [ ] M — Buffers SoA para `f_i`, flags, parâmetros por elemento (uniform/storage); kernel WGSL stream-collide D2Q9 TRT.
- [ ] M — BCs em WGSL (tabela de elementos indexada por flags, sem ramos divergentes desnecessários).
- [ ] M — Térmico D2Q5 em WGSL.
- [ ] M — Reduções em GPU (diagnósticos, forças) e kernel de amostragem/conversão para f16 antes de copiar para CPU.
- [ ] P — Leitura assíncrona com double-buffering de staging (a simulação não pára enquanto se copia um frame).
- [ ] M — Testes de paridade GPU vs CPU (tolerância em [04 §7](04-metodos-numericos.md#7-validação)); executam se houver adaptador (incluindo software, ex.: lavapipe/WARP no CI).
- [ ] P — Benchmarks em várias resoluções; registo em `docs/benchmarks.md`.
- [ ] M — (Opcional) Padrão AA / Esoteric Pull para reduzir memória a metade.

### 2b — CUDA (`crates/cfd-cuda`, feature Cargo `cuda`)

- [ ] P — Crate `cfd-cuda` com `cudarc`; compilação condicional (`--features cuda`) para o workspace compilar sem CUDA Toolkit; deteção do dispositivo e da *compute capability*.
- [ ] M — Kernels CUDA C (`kernels/*.cu`) compilados por NVRTC em runtime (ou PTX pré-compilado no `build.rs`): stream-collide D2Q9 TRT, BCs, térmico D2Q5 — tradução direta dos kernels WGSL, mesma indexação.
- [ ] M — Reduções (diagnósticos, forças) com warp shuffles; amostragem para f16; cópias assíncronas com streams e memória *pinned*.
- [ ] P — Implementar o trait `Backend` → `BackendKind::Cuda`; `backend: "auto"` passa a preferir CUDA > wgpu > CPU (configurável).
- [ ] M — Testes de paridade CUDA vs CPU (mesma tolerância); executam só com `--features cuda` e GPU NVIDIA (job de CI *self-hosted* ou manual).

### 2c — Comparação entre engines

- [ ] M — `cfd-cli compare scene.json --backends cpu,wgpu,cuda --steps N`: executa a mesma cena em cada backend e reporta diferenças por campo (máx. absoluta, L2 relativa, mapa de diferenças em PNG), diagnósticos (massa, energia) e MLUPS.
- [ ] P — `cfd-cli bench --backends wgpu,cuda`: tabela de MLUPS por resolução (256² → 4096²), f32 e (mais tarde) f16; resultados em `docs/benchmarks.md`.
- [ ] P — Na API/UI (Fases 3–5): o backend escolhido aparece nos metadados do job; possibilidade de lançar o mesmo cenário em dois backends e ver as diferenças lado a lado (reaproveita a UI de comparação).

**Aceitação:** paridade CPU↔wgpu↔CUDA dentro da tolerância; ≥ 1 000 MLUPS numa GPU de gama média em 1024² em ambas as engines; `cfd-cli compare` produz relatório para os casos de validação; fallback CUDA → wgpu → CPU testado.

---

## Fase 3 — Servidor e jobs

**Objetivo:** pedidos de simulação via HTTP com streaming de resultados.

- [ ] M — `cfd-server` com axum: rotas de [03 §7](03-modelo-de-dados.md#7-api-rascunho), limites de tamanho de corpo, CORS, tracing.
- [ ] M — Validação de cena no servidor (schema + regras de consistência + estabilidade) com mensagens localizáveis.
- [ ] M — Job manager: fila em memória, estados (`queued/running/paused/completed/failed/cancelled`), semáforo por dispositivo, cancelamento cooperativo.
- [ ] M — Worker loop numa thread dedicada (o solver não bloqueia o runtime tokio); canal para frames. O loop processa **comandos** (`Step`, `Pause`, `Resume`, `Cancel`, `PatchCells`, `SetParam`) entre lotes de passos — base do modo interativo ([ADR-011](07-decisoes-e-perguntas.md)).
- [ ] M — WebSocket: progresso (JSON) + frames (binário); backpressure (se o cliente for lento, descartar frames intermédios de *streaming* mas guardar todos em disco).
- [ ] M — `ResultStore` em disco; SQLite para metadados de jobs e cenas.
- [ ] P — `/api/capabilities` (backends, GPU, limites).
- [ ] P — Testes de integração: submeter cena → receber N frames → `completed`.

**Aceitação:** um cliente de teste (script) submete o cilindro e recebe frames em tempo real; cancelar funciona em < 1 s.

---

## Fase 4 — Editor web (quadro de desenho)

**Objetivo:** desenhar e configurar cenas no browser.

- [ ] M — Canvas com zoom/pan, grelha de pixels, camadas `Uint8Array/Uint16Array`, renderização com cores por tipo/elemento.
- [ ] M — Ferramentas: lápis (tamanho de pincel), borracha, linha, retângulo, elipse, balde, seleção/mover/copiar.
- [ ] M — Paleta de tipos (fluido, parede, inlet, outlet, fonte de calor, região de líquido, sonda); criação automática de elemento ao desenhar uma nova região; seleção de elemento existente.
- [ ] M — Painel de propriedades por elemento (velocidade, térmica, material) e propriedades globais (domínio, escala, fluidos, gravidade, tempo, método, backend).
- [ ] P — Biblioteca de fluidos/materiais pré-definidos (ar, água, óleo; alumínio, cobre, vidro…).
- [ ] M — Undo/redo, guardar/carregar (ficheiro local `.cfdscene` + servidor), importar PNG como máscara.
- [ ] M — Validação no cliente e estimativas (Re, memória, tempo estimado) antes de submeter.
- [ ] P — Galeria de exemplos.

**Aceitação:** desenhar o cilindro em canal do zero em < 2 min e submeter sem erros.

---

## Fase 5 — Visualização e reprodução → **MVP**

**Objetivo:** ver e analisar os resultados.

- [ ] M — Cliente WebSocket, descodificação de frames (zstd em WASM ou `fzstd`), buffer de frames em memória/IndexedDB.
- [ ] M — Renderização WebGL2: texturas float16, colormaps (viridis, coolwarm, …), escala automática/manual, legenda, máscara de sólidos.
- [ ] M — Seletor de campo (velocidade, pressão, temperatura, vorticidade), vetores, linhas de corrente (RK4 no cliente).
- [ ] M — Partículas traçadoras/fumo animados (GPU no cliente).
- [ ] P — Linha temporal: play/pause/scrub/velocidade.
- [ ] M — Gráficos de sondas e forças (ex.: uPlot); estatísticas (St, C_D, Nu).
- [ ] P — Exportar PNG, CSV, e vídeo (WebCodecs/MediaRecorder).

- [ ] M — Galeria de **exemplos rápidos** (cilindro, convecção natural, dam break…) com resultados pré-calculados ou baixa resolução (< 10 s), para educação/demo ([ADR-009](07-decisoes-e-perguntas.md)).

---

## Fase 5b — Modo interativo (pós-MVP, [ADR-011](07-decisoes-e-perguntas.md))

- [ ] M — `PatchCells` nos backends (CPU, wgpu, CUDA): reinicializar `f_i` das células que mudam de tipo (sólido → fluido com equilíbrio local; fluido → sólido).
- [ ] M — `SetParam` em tempo real (velocidade de inlet, temperatura, gravidade), com nova verificação de estabilidade.
- [ ] M — UI: desenhar sobre a simulação a correr; envio de patches pelo WebSocket; frames a ≥ 30 fps em 512².
- [ ] P — Modo "sandbox" na galeria de exemplos (ótimo para educação e portfólio).

## Fase 6 — Líquidos (superfície livre — **parte do MVP**) e multifásico

### 6a — Superfície livre (MVP, [ADR-008](07-decisoes-e-perguntas.md))
- [ ] G — LBM com superfície livre em CPU (massa por célula, fração de enchimento, células de interface, reconstrução de `f_i` na interface, conversão de tipos de célula).
- [ ] G — Port para wgpu e CUDA (conversão de células em passos separados sem condições de corrida); paridade com CPU via `cfd-cli compare`.
- [ ] M — Editor: ferramenta "região de líquido inicial", nível de enchimento, escolha do líquido.
- [ ] M — Visualização da interface (contorno marching squares, fração de volume, cor de água).
- [ ] P — Validação: dam break (Martin & Moyce 1952), conservação de massa do líquido < 0.1%.

### 6b — Pós-MVP
- [ ] M — Tensão superficial (curvatura por PLIC/altura) e ângulo de contacto por parede.
- [ ] G — Phase-field LBM (dois fluidos com rácio de densidade elevado).
- [ ] P — Validação: Rayleigh-Taylor, bolha ascendente (Hysing et al. 2009).

**Aceitação do MVP (Fases 0–5 + 6a):** fluxo completo desenhar → simular em GPU (wgpu e CUDA) → ver em streaming → exportar, para gases (cilindro, cavidade aquecida) **e líquidos (dam break)**, com resultados validados.

## Fase 7 — Métodos alternativos

- [ ] G — Solver de projeção em grelha MAC (CPU+GPU) com multigrid geométrico para Poisson.
- [ ] G — FLIP/APIC sobre o solver de projeção.
- [ ] G — Volumes finitos compressível (MUSCL + HLLC + RK2/RK3), BCs supersónicas.
- [ ] M — UI de comparação lado a lado de métodos no mesmo cenário.

## Fase 8 — Otimizações avançadas

Detalhadas em [06](06-otimizacoes-e-ia.md): f16 para armazenamento, grelhas esparsas, AMR quadtree, multi-GPU, passo de tempo adaptativo, LES, backend CUDA opcional.

## Fase 9 — IA

Detalhada em [06 §3](06-otimizacoes-e-ia.md#3-ia): geração de dataset com o próprio solver, modelos substitutos (U-Net/FNO), warm start, super-resolução, Poisson acelerado por IA.

## Fase 9b — Outros estados da matéria

Objetivo: **todas as mudanças de estado**, incluindo materiais granulares (ex.: gelo picado a derreter num copo). Abordagem proposta em [ADR-012](07-decisoes-e-perguntas.md).

| Transição | Exemplo | Método proposto |
|-----------|---------|-----------------|
| Sólido ↔ líquido (contínuo) | Bloco de gelo a derreter, metal a solidificar | Método de entalpia + LBM (zona pastosa como meio poroso) |
| **Granular ↔ líquido** | **Gelo picado a derreter** | MPM com temperatura e calor latente por partícula; massa derretida → líquido |
| Líquido ↔ gás | Ebulição, evaporação, condensação | LBM multifásico térmico (pseudo-potencial ou phase-field com fonte de massa) |
| Sólido ↔ gás *(Fase 9c)* | Sublimação (gelo seco), deposição (geada) | Entalpia + transferência de massa na interface |
| Gás ↔ plasma *(Fase 9c)* | Ionização | Depende da Fase de plasma |

- [ ] G — Método de entalpia (fusão/solidificação) com validação: problema de Stefan (solução analítica) e fusão de gálio numa cavidade (Gau & Viskanta 1986).
- [ ] G — **Granular:** MPM 2D (CPU → GPU) com reologia granular (Drucker-Prager), acoplado ao fluido; validação: colapso de coluna granular.
- [ ] G — Acoplamento térmico MPM ↔ fluido com calor latente → gelo picado a derreter.
- [ ] G — Evaporação/condensação (LBM multifásico térmico); validação: problema de Stefan líquido-vapor.

## Fase 9c — Transições raras (fase seguinte à 9b)

- [ ] M — Sublimação (sólido → gás, ex.: gelo seco) e deposição (gás → sólido, ex.: geada): entalpia + transferência de massa na interface.
- [ ] M — Outros casos raros a avaliar: sobrefusão (líquido abaixo do ponto de fusão), ponto triplo, fluidos supercríticos, ionização (gás → plasma, depende da fase de plasma).
- [ ] G — **Plasma:** começar por MHD resistivo 2D (fluido condutor + campo magnético) ou modelo de dois fluidos simplificado; requer solver compressível (Fase 7).

## Fase 10 — **Etapa 2: 3D** e malhas modernas ([ADR-004](07-decisoes-e-perguntas.md))

Só começa quando o 2D estiver completo e validado. É praticamente um segundo projeto na UI.

- [ ] G — Solvers D3Q19/D3Q27 (reusando a indexação genérica preparada no 2D) nos três backends; validação 3D (cavidade cúbica, esfera, Taylor-Green 3D).
- [ ] G — Editor 3D: extrusão de desenhos 2D, edição por camadas (fatias), primitivas (caixa, cilindro, esfera), importação de STL voxelizado. Three.js via react-three-fiber.
- [ ] G — Visualização 3D: cortes, isosuperfícies, volume rendering, linhas de corrente 3D.
- [ ] M — Memória: com 8 GB de VRAM, 3D a ~256³–384³ exige f16 storage e AA-pattern (Fase 8) — por isso a Fase 8 é pré-requisito.
- [ ] G — Malhas não estruturadas / cut-cell / immersed boundary para geometria suave (ver [06](06-otimizacoes-e-ia.md)).
- [ ] G — SPH como alternativa sem malha.

## Transversal — Self-hosting (baixa prioridade, [ADR-010](07-decisoes-e-perguntas.md))

- [ ] `cfd-server` serve o frontend embebido; `--open` abre o browser (pode entrar já na Fase 3).
- [ ] Demo estática no GitHub Pages com exemplos pré-calculados (custo zero).
- [ ] Autenticação, quotas e rate limiting (desligados por omissão).
- [ ] Fila distribuída (Redis/NATS), vários workers/GPUs.
- [ ] Postgres + S3/MinIO.
- [ ] Containers com Vulkan, deploy, observabilidade (Prometheus/Grafana).
- [ ] Documentação do utilizador e tutoriais.

---

## Riscos principais

| Risco | Impacto | Mitigação |
|-------|---------|-----------|
| Instabilidade LBM a Re elevado | Simulações a explodir | TRT/MRT, LES, avisos de estabilidade, aumentar resolução automaticamente |
| Limites de buffers em wgpu/WebGPU (ex.: 128 MB–2 GB por binding) | Domínios grandes falham | Dividir `f_i` em vários buffers; consultar limites do adaptador |
| Largura de banda para streaming de frames | UI lenta | f16 + zstd + subamostragem + descartar frames intermédios |
| Diferenças numéricas GPU/CPU | Testes instáveis | Tolerâncias relativas, testes em campos agregados |
| Âmbito demasiado grande | Projeto nunca chega a MVP | MVP estrito (Fases 0–5); tudo o resto é pós-MVP |
