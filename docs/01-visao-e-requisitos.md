# 01 — Visão e requisitos

## 1. Visão

Uma aplicação web onde qualquer pessoa (estudante, engenheiro, curioso) consegue **desenhar um problema de escoamento em minutos** e obter uma simulação fisicamente credível, calculada num servidor com GPU. A grelha de pixels é ao mesmo tempo a interface de desenho e a grelha de cálculo, o que elimina o passo de geração de malha na primeira versão.

A longo prazo o projeto deve servir de **plataforma de experimentação de métodos numéricos**: vários solvers atrás de uma interface comum, comparáveis no mesmo cenário, com otimizações modernas (malhas adaptativas, multi-GPU, IA).

**Contexto (2026-10-09):** projeto de portfólio e exploração pessoal, **open source e público**, com foco em educação e exemplos de simulação rápida. MVP cobre **gases e líquidos** em 2D; depois vêm modo interativo, granular, plasma e todas as mudanças de estado (incl. granular → líquido); o **3D é a Etapa 2**, após o 2D completo e validado. GPU de desenvolvimento: NVIDIA GeForce RTX 4060, 8 GB. Licença: MIT OR Apache-2.0.

## 2. Objetivos

1. **Simplicidade de entrada:** desenhar → configurar → simular, sem conhecimentos de geração de malhas.
2. **Desempenho:** simulações 2D de 512×512 em tempo quase real numa GPU de consumo; 2048×2048 em modo batch.
3. **Rigor verificável:** cada solver validado contra casos de referência publicados (ver [04](04-metodos-numericos.md#7-validação)).
4. **Extensibilidade:** novos métodos, novos tipos de fronteira e novos fluidos sem alterar o frontend nem o protocolo.
5. **Portabilidade:** GPU de qualquer fabricante (Vulkan/Metal/DX12) e fallback CPU automático.

## 3. Não-objetivos (por agora)

- Geometria CAD importada (STEP/IGES). Possível mais tarde via rasterização para a grelha.
- 3D na primeira fase — o formato de dados é desenhado para o permitir (voxels), mas o MVP é 2D.
- Certificação para uso industrial.
- Multi-tenant em escala com faturação.

## 4. Utilizadores e casos de uso

| Persona | Caso de uso típico |
|---------|--------------------|
| Estudante | Ver o efeito do número de Reynolds no escoamento à volta de um cilindro (esteira de von Kármán) |
| Engenheiro / maker | Avaliar o arrefecimento de uma caixa com ventoinha e componentes quentes |
| Investigador / developer | Comparar LBM vs. projeção no mesmo cenário; testar um modelo de IA substituto |
| Curioso | Brincar com fumo, água a cair, convecção natural |

## 5. Requisitos funcionais

### 5.1 Editor (frontend)
- RF-01 Criar um domínio com dimensão W×H (pixels/células) e escala física (metros por pixel).
- RF-02 Ferramentas de desenho: lápis, borracha, linha, retângulo, elipse, balde (flood fill), seleção/mover, importar imagem (PNG → máscara).
- RF-03 Paleta de **tipos de célula**: fluido, sólido/parede, entrada (inlet), saída (outlet), fonte de calor, região de fluido inicial (ex.: água), sonda (ponto de medição).
- RF-04 Painel de propriedades por **elemento de fronteira** (cada região desenhada com o mesmo ID): condição de velocidade (no-slip, slip, velocidade imposta), condição térmica (temperatura fixa, fluxo de calor, adiabática), material do sólido (condutividade, se houver condução no sólido).
- RF-05 Definir fluidos: escolher de uma biblioteca (ar, água, óleo…) ou propriedades personalizadas (densidade, viscosidade, condutividade, capacidade térmica, coeficiente de expansão).
- RF-06 Parâmetros globais: gravidade, tempo físico total, intervalo de output, método numérico, precisão (f32/f16/f64), preferência de backend (auto/GPU/CPU).
- RF-07 Validação no cliente antes de enviar (ex.: domínio sem saída com entrada de massa → aviso; Mach/estabilidade estimados).
- RF-08 Guardar/carregar cenas (local e no servidor), undo/redo, exemplos pré-definidos.

### 5.2 Simulação (servidor)
- RF-10 Receber pedidos de simulação, validar, colocar em fila e devolver um ID de job.
- RF-11 Escolher automaticamente o backend (GPU se disponível, senão CPU).
- RF-12 Enviar progresso e frames de resultados em streaming (WebSocket).
- RF-13 Cancelar, pausar e retomar jobs.
- RF-14 Guardar resultados (campos e séries temporais das sondas) para reprodução e download.
- RF-15 (Futuro) Modo interativo: alterar parâmetros/geometria com a simulação a correr.

### 5.3 Visualização
- RF-20 Mapas de cor de campos: velocidade (magnitude e componentes), pressão, temperatura, densidade, fração de fase, vorticidade.
- RF-21 Linhas de corrente, vetores (glifos), partículas traçadoras, fumo/corante.
- RF-22 Reprodução temporal (play/pause/scrub), escala de cores ajustável, legendas.
- RF-23 Gráficos das sondas ao longo do tempo; estatísticas (forças de arrasto/sustentação, número de Strouhal, Nusselt).
- RF-24 Exportar: PNG/MP4/GIF, CSV das sondas, campos em formato binário/VTK.

## 6. Requisitos não funcionais

| ID | Requisito | Meta inicial |
|----|-----------|--------------|
| RNF-01 | Desempenho GPU (LBM D2Q9, f32) | ≥ 2 000 MLUPS na RTX 4060 (wgpu e CUDA). Largura de banda 272 GB/s ÷ 72 B/célula/passo (9 leituras + 9 escritas f32) ⇒ teto teórico ~3 700 MLUPS |
| RNF-02 | Desempenho CPU | ≥ 50 MLUPS em 8 núcleos |
| RNF-03 | Latência até ao primeiro frame | < 2 s para 512×512 |
| RNF-04 | Tamanho máximo do domínio (MVP) | 4096×4096 (~1.4 GB em f32; cabe em 8 GB de VRAM com margem para térmico e superfície livre) |
| RNF-05 | Reprodutibilidade | Mesmo pedido + mesma versão do solver ⇒ resultados determinísticos no backend CPU; diferença GPU vs CPU < tolerância definida |
| RNF-06 | Segurança | Validação estrita do input, limites de tamanho/tempo por job, sem execução de código do utilizador |
| RNF-07 | Observabilidade | Logs estruturados, métricas (jobs/s, MLUPS, uso de GPU), tracing |
| RNF-08 | Portabilidade | Windows, Linux; GPUs NVIDIA/AMD/Intel/Apple via wgpu |

*MLUPS = milhões de atualizações de célula por segundo (métrica padrão em LBM).*

## 7. Glossário

- **Célula / pixel:** unidade da grelha de cálculo; no MVP 1 pixel = 1 célula.
- **Fronteira (boundary):** conjunto de células com o mesmo tipo e ID de elemento, partilhando propriedades.
- **Backend:** implementação concreta de um solver para um tipo de hardware (GPU/CPU).
- **Job:** um pedido de simulação em execução ou em fila.
- **Frame:** snapshot dos campos num instante de output.
