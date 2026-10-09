# 06 — Otimizações avançadas e IA

Tudo aqui é **pós-MVP**, salvo indicação. Ordenado aproximadamente por relação benefício/custo.

## 1. Otimizações de desempenho

### 1.1 Memória e largura de banda (LBM é limitado por largura de banda)
| Técnica | Ganho esperado | Notas |
|---------|----------------|-------|
| Kernel fundido stream-collide + SoA | Base | Já no MVP |
| **AA pattern / Esoteric Pull** | −50% memória, até +30% velocidade | Um único buffer de `f_i` |
| **Armazenamento em f16/"FP16C"** (cálculo em f32) | −50% memória, ~1.5–2× velocidade | Técnica do FluidX3D; validar precisão |
| Flags compactas + pré-computação de vizinhos sólidos | Menos divergência | Kernels sem ramos |
| Grelha **esparsa por blocos** (só blocos com fluido) | Grande em domínios com muito sólido | Tiles 16×16/32×32 + tabela de blocos ativos |

### 1.2 Algoritmos
- **Multigrid** geométrico/algébrico para Poisson (solver de projeção, FLIP).
- **Passo de tempo adaptativo** (CFL) nos métodos explícitos que o permitem; em LBM, reescalar `u_lb`.
- **Refinamento local (AMR)** em quadtree/octree: LBM multi-nível (refinamento de rede com reescalonamento de `τ`) — células finas junto a paredes e esteiras, grossas no resto.
- **Mesh dinâmica:** refinamento/derrefinamento periódico guiado por vorticidade, gradientes de temperatura ou interface de fase.
- **Turbulência:** LES (Smagorinsky, WALE) e leis de parede para Re elevados a baixa resolução.

### 1.3 Hardware
- **Multi-GPU:** decomposição de domínio em faixas/blocos com troca de halos (1 célula em LBM); depois multi-nó (MPI ou gRPC/QUIC).
- **Otimizações específicas CUDA** (o backend CUDA base já existe desde a Fase 2): shared memory para *tiling*, warp shuffles, Tensor Cores para inferência IA, CUDA Graphs para reduzir overhead de lançamento, multi-GPU com NCCL/peer-to-peer.
- **CPU:** SIMD explícito (AVX2/AVX-512/NEON), *tiling* para cache, NUMA-awareness.
- **Simulação no browser** (WebGPU): o mesmo WGSL a correr localmente para pré-visualizações rápidas de baixa resolução, sem servidor.

## 2. Alternativas à grelha uniforme ("variações mais modernas")

| Representação | Quando usar | Integração com o editor de pixels |
|---------------|-------------|-----------------------------------|
| Grelha uniforme (MVP) | Sempre como base | Direta |
| **Quadtree/Octree AMR** | Resolução variável | Pixels definem a geometria no nível mais fino |
| **Immersed Boundary / interpolated bounce-back** | Paredes curvas sem "escadas" | Extrair contorno sub-pixel dos pixels (marching squares + suavização) |
| **Cut-cell** (volumes finitos) | Precisão em paredes com grelha cartesiana | Igual ao anterior |
| **Malha não estruturada** (triângulos/polígonos; FV ou DG) | Geometria complexa, refinamento anisotrópico | Contorno → gerador de malha (Delaunay restrito; ex.: `spade`) |
| **Sem malha** (SPH, MPM) | Líquidos violentos, granular, sólidos deformáveis | Pixels → partículas iniciais |
| **Métodos de alta ordem** (DG, spectral element) | Precisão por grau de liberdade | Depende de malha não estruturada |

O editor continua pixel a pixel; a **conversão para outra representação** acontece no servidor (`Scene → Domain` passa a ter várias implementações). Opcionalmente, ferramentas vetoriais no editor (curvas Bézier) para geometria suave.

## 3. IA

### 3.1 Pipeline de dados
1. Gerador procedural de cenas (obstáculos aleatórios, inlets, temperaturas) em `tools/datagen`.
2. Execução em massa com o solver GPU (modo batch, sem streaming) → dataset de pares (cena, campos ao longo do tempo).
3. Armazenamento em Zarr/HDF5 com metadados (Re, Ra, método, resolução).
4. Treino offline em Python/PyTorch; exportação para ONNX.
5. Inferência no worker Rust via `ort` (ONNX Runtime, CUDA/DirectML/CPU) ou `burn`.

### 3.2 Usos, do mais seguro ao mais ambicioso
| Uso | Descrição | Risco para o rigor |
|-----|-----------|--------------------|
| **Warm start** | IA prevê o estado estacionário aproximado; o solver clássico converge a partir daí | Nenhum (o solver corrige) |
| **Pré-visualização instantânea** | Modelo substituto mostra uma previsão em < 100 ms enquanto o utilizador desenha | Rotulado como "previsão IA" |
| **Pré-condicionador / Poisson aprendido** | Rede acelera o passo mais caro dos métodos de projeção | Baixo (iterações finais clássicas) |
| **Super-resolução** | Simular em grelha grossa e reconstruir detalhe | Médio |
| **Closure de turbulência aprendida** | Modelo de sub-malha treinado em simulações finas | Médio/alto |
| **Substituto completo** (FNO, U-Net, MeshGraphNets, transformers de operadores) | Avança vários passos de uma vez | Alto — sempre com indicador de confiança e opção de "verificar com solver" |

### 3.3 Arquiteturas candidatas
- **U-Net** condicionada pela máscara de geometria e parâmetros (Re, Ra) — simples e forte em grelhas.
- **Fourier Neural Operator (FNO)** — independente da resolução.
- **MeshGraphNets / GNN** — para malhas não estruturadas e AMR.
- **Modelos de difusão** para super-resolução e geração de campos turbulentos.

### 3.4 Avaliação
Métricas físicas e não só de pixel: conservação de massa, divergência, erro em C_D/St/Nu nos casos de validação, estabilidade em rollouts longos. O modelo só é ativado por omissão se cumprir limites definidos.

## 4. Priorização sugerida pós-MVP

1. f16 storage + AA pattern (ganho grande, risco baixo)
2. LES + TRT/MRT robusto (desbloqueia Re realistas)
3. Superfície livre (líquidos — muito pedido visualmente)
4. Grelha esparsa por blocos
5. Warm start e pré-visualização por IA
6. AMR quadtree
7. Multi-GPU
8. Malhas não estruturadas / 3D
