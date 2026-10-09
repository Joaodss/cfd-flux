# 07 — Decisões (ADRs) e perguntas em aberto

## Registo de decisões

Formato curto: contexto → decisão → consequências. Estado: **Proposta** (a confirmar pelo dono do projeto) ou **Aceite**.

### ADR-001 — Rust para servidor e solver · *Proposta*
- **Contexto:** desempenho próximo de C/C++, segurança de memória, bom ecossistema web (axum) e GPU (wgpu).
- **Decisão:** Rust para todo o backend.
- **Consequências:** menos bibliotecas CFD prontas do que em C++/Python; compensado por escrevermos os kernels de raiz. Python apenas para treino de IA e scripts de análise.

### ADR-002 — Dois backends GPU desde o início: wgpu/WGSL e CUDA · *Aceite (2026-10-09)*
- **Contexto:** queremos suportar qualquer GPU e possivelmente correr no browser, mas a GPU de desenvolvimento do autor é NVIDIA e há interesse em comparar engines.
- **Decisão:** implementar na Fase 2 **ambos** os backends GPU: `wgpu` + WGSL (portável) e CUDA via `cudarc` (kernels CUDA C compilados em runtime com NVRTC, ou PTX pré-compilado). O CUDA fica atrás de uma *feature* Cargo `cuda` para o projeto compilar sem o CUDA Toolkit.
- **Consequências:** três implementações do mesmo kernel (CPU, WGSL, CUDA) → mais manutenção, compensada por testes de paridade cruzados e por uma ferramenta de comparação (`cfd-cli compare`). Permite medir o custo real da portabilidade (MLUPS wgpu vs CUDA) e usar funcionalidades CUDA (shared memory, warp shuffles) onde valer a pena.

### ADR-003 — LBM como primeiro método · *Proposta*
- **Contexto:** grelha de pixels, fronteiras arbitrárias desenhadas à mão, GPU.
- **Decisão:** LBM D2Q9 (TRT) + D2Q5 térmico no MVP.
- **Consequências:** limitado a escoamentos de Mach baixo no MVP; gases compressíveis a alta velocidade ficam para a Fase 7.

### ADR-004 — 2D completo primeiro; 3D como segunda etapa do projeto · *Aceite (2026-10-09)*
- **Contexto:** 3D é um objetivo real a médio prazo (Q8), mas muda bastante a UI (edição por voxels/camadas, visualização volumétrica).
- **Decisão:** todo o 2D (incluindo validação) é concluído e bem testado antes de começar o 3D. Mesmo assim, desde já: `grid` aceita `depth` opcional, camadas são tensores N-dimensionais, e o código do solver é genérico na dimensão onde não custa (ex.: tabelas de velocidades `DxQy` parametrizadas, indexação via função `idx()`), para o 3D não exigir reescrita.
- **Consequências:** o 3D passa de "talvez" para a **Etapa 2** do roadmap (ver plano).

### ADR-005 — Frontend em TypeScript + Vite + React · *Aceite (2026-10-09)*
- **Contexto:** o autor conhece React; o editor de canvas e a visualização WebGL são código imperativo, independente do framework, por isso o framework pesa pouco no desempenho.
- **Decisão:** React + TypeScript + Vite, com: **Zustand** (estado, fácil de usar fora de componentes React — importante para o canvas), **Radix UI / shadcn/ui + Tailwind** (painéis e formulários), **uPlot** (gráficos rápidos de séries temporais), **Vitest** + **Playwright** (testes). Para o 3D (Etapa 2): **Three.js via react-three-fiber** — razão adicional para ficar em React.
- **Regra:** os pixels nunca passam pelo estado React; o canvas lê/escreve `TypedArray`s diretamente e o React só renderiza painéis e ferramentas.
- **Alternativas consideradas:** Svelte/SolidJS (menos overhead, mas sem ganho real aqui e curva de aprendizagem extra).

### ADR-006 — Servidor e worker no mesmo processo no MVP · *Proposta*
- **Decisão:** fila em memória; separação em processos só quando houver várias GPUs/utilizadores.
- **Consequências:** deploy trivial; o trait da fila permite trocar por Redis/NATS sem mexer no resto.

### ADR-007 — Resultados em f16 + zstd para streaming · *Proposta*
- **Decisão:** frames de streaming em f16 comprimidos; downloads em f32.

### ADR-008 — Líquidos com superfície livre no MVP · *Aceite (2026-10-09)*
- **Contexto:** o autor quer gases e líquidos desde a fase inicial (Q5).
- **Decisão:** o MVP inclui **LBM com superfície livre** (água/ar como líquido + "vazio"), que reutiliza o kernel D2Q9 e os três backends. Phase-field (dois fluidos reais), tensão superficial avançada e FLIP ficam pós-MVP.
- **Consequências:** o MVP cresce (~2–3 semanas de trabalho extra); a superfície livre é mais difícil de paralelizar em GPU (conversão de células interface ↔ fluido ↔ gás), por isso entra só depois de o LBM monofásico estar validado nas três engines.

### ADR-009 — Open source e orientado a exemplos · *Aceite (2026-10-09)*
- **Contexto:** portfólio, público, uso educativo (Q1).
- **Decisão:** repositório público desde a Fase 0; galeria de exemplos rápidos (pré-calculados ou de baixa resolução, < 10 s) como funcionalidade de primeira classe; documentação de utilizador e README com GIFs.
- **Consequências:** CI tem de funcionar sem GPU/CUDA; cuidado com limites de recursos se houver uma demo pública alojada.

### ADR-010 — Local-first, self-hosting fácil, sem servidor alojado · *Aceite (2026-10-09)*
- **Contexto:** sem budget para servidor; uso na máquina de desenvolvimento; outros podem querer alojar (Q3).
- **Decisão:** `cfd-server` é **um único binário** que serve também o frontend estático (`cfd-server --open` abre o browser). Sem dependências externas obrigatórias (SQLite + disco). `Dockerfile`/`docker-compose` opcionais para quem quiser alojar. Autenticação/quotas ficam desligadas por omissão e só ativas por configuração.
- **Sugestão (proposta, por decidir):** demo pública gratuita no **GitHub Pages** com a galeria de exemplos pré-calculados e, mais tarde, o solver compilado para WebGPU no browser em baixa resolução — custo zero de servidor.
- **Consequências:** a secção "Transversal — Produção" do plano passa a "Self-hosting" e baixa de prioridade.

### ADR-011 — Arquitetura preparada para modo interativo desde o início · *Aceite (2026-10-09)*
- **Contexto:** o modo interativo (alterar geometria/parâmetros com a simulação a correr) é desejado mas não é MVP (Q6). Como tudo corre localmente (ADR-010), não há latência de rede — o modo interativo é viável e muito vistoso para portfólio.
- **Decisão:** o MVP **não** tem UI interativa, mas desde a Fase 3 o ciclo do worker é um *loop de comandos* (`Step`, `Pause`, `Resume`, `Cancel`, `PatchCells`, `SetParam`) e o trait `Solver` tem `update_boundaries`. Assim pausar/retomar do MVP já usa o mesmo mecanismo, e o modo interativo (Fase 5b) é só UI + implementação de `PatchCells` nos kernels.
- **Consequências:** custo quase nulo agora; evita reescrever o worker depois.

### ADR-012 — MPM como candidato para mudanças de estado com sólidos/granulares · *Proposta*
- **Contexto:** queremos todas as mudanças de estado, incluindo granular → líquido (ex.: gelo picado a derreter) (Q9).
- **Decisão proposta:** solid/granular via **MPM** (Material Point Method) acoplado à grelha, com temperatura e calor latente por partícula; derretimento transfere massa da partícula para o líquido (LBM superfície livre ou grelha MPM). Fusão/solidificação de volumes contínuos com método de entalpia; evaporação/condensação com LBM multifásico térmico.
- **Referência de partida:** Stomakhin et al., "Augmented MPM for phase-change and varied materials", SIGGRAPH 2014.
- **A decidir na Fase 9b** após protótipo (alternativa: DEM + LBM).

## Perguntas em aberto

Respostas a estas perguntas podem alterar prioridades. Registar a resposta e a data aqui.

| # | Pergunta | Default assumido até haver resposta |
|---|----------|------------------------------------|
| Q1 | Qual é o público principal: uso pessoal/educativo, ou um serviço público com vários utilizadores? | **Respondido (2026-10-09):** projeto de portfólio e exploração; **open source e público**; útil para educação e para exemplos de simulação rápida. Multi-utilizador em produção continua tardio |
| Q2 | Que GPU(s) tens disponíveis para desenvolvimento e para o servidor (fabricante, VRAM)? | **Respondido (2026-10-09):** **NVIDIA GeForce RTX 4060, 8 GB** (confirmado com `nvidia-smi`, driver 610.62). CUDA Toolkit ainda não instalado |
| Q3 | Onde vai correr o servidor: máquina local, servidor próprio, cloud? | **Respondido (2026-10-09):** só na máquina de desenvolvimento; sem budget para servidor. Deve ser fácil para terceiros fazerem self-hosting (ver ADR-010) |
| Q4 | Precisas de precisão de engenharia (validação quantitativa) ou o foco é visual/interativo? | **Respondido (2026-10-09):** sim, validação de precisão quantitativa; método explicado em [08 — Guia de validação](08-guia-validacao.md) |
| Q5 | Prioridade entre gases (aerodinâmica, ventilação, arrefecimento) e líquidos (superfície livre)? | **Respondido (2026-10-09):** **gases e líquidos na fase inicial** → líquidos com superfície livre entram no MVP (ver ADR-008) |
| Q6 | Modo interativo (alterar geometria com a simulação a correr) é importante cedo? | **Respondido (2026-10-09):** desejado, não no MVP → arquitetura preparada desde a Fase 3 (ADR-011), UI na Fase 5b |
| Q7 | Preferência de framework frontend (React, Svelte, outro)? | **Respondido (2026-10-09):** React (ADR-005, com ferramentas recomendadas) |
| Q8 | 3D é um objetivo real a médio prazo ou só "talvez"? | **Respondido (2026-10-09):** objetivo real; **Etapa 2**, depois de o 2D estar completo e testado (ADR-004) |
| Q9 | "Outros estados" — o que tens em mente: plasma, granular, sólidos deformáveis, mudança de fase? | **Respondido (2026-10-09):** pós-MVP: granular, plasma e mudanças de estado comuns, incluindo granular → líquido (gelo picado a derreter) (ADR-012). Casos raros (sólido ↔ gás: sublimação/deposição, e outros) numa fase seguinte (Fase 9c) |
| Q10 | Licença do projeto — open source confirmado; qual licença? | **Respondido (2026-10-09):** **MIT OR Apache-2.0** |
| Q11 | Experiência prévia com Rust, GPU e CFD (para calibrar o detalhe da documentação e o ritmo)? | **Respondido (2026-10-09):** muita experiência em CFD; pouca em Rust e GPU (aprende rápido); background C#. → Fase 0 inclui protótipos de aprendizagem (ver plano) |

### Opções de licença (Q10)

| Licença | Tipo | O que permite / obriga | Adequada se… |
|---------|------|------------------------|--------------|
| **MIT** | Permissiva | Qualquer uso, incluindo comercial e fechado; só exige manter o aviso de copyright | Queres a máxima adoção, sem condições |
| **Apache-2.0** | Permissiva | Como MIT + concessão explícita de patentes e regras para contribuições | Igual, com proteção de patentes |
| **MIT OR Apache-2.0** | Permissiva (dupla) | Quem usa escolhe uma; padrão do ecossistema Rust | Projeto Rust que quer integrar-se sem fricção |
| **MPL-2.0** | Copyleft fraco (por ficheiro) | Ficheiros modificados têm de continuar abertos; pode ser combinado com código fechado | Meio-termo |
| **GPL-3.0** | Copyleft forte | Quem **distribuir** versões modificadas tem de abrir o código inteiro sob GPL | Queres garantir que derivados distribuídos ficam abertos |
| **AGPL-3.0** | Copyleft forte + rede | Como GPL, mas também quem **disponibiliza como serviço web** uma versão modificada tem de abrir o código | Queres impedir que alguém feche o projeto e o venda como SaaS |
