# 04 — Métodos numéricos

Este documento descreve os métodos a implementar, por ordem, e porquê. Todos ficam atrás do trait `Solver` (ver [02](02-arquitetura.md#24-núcleo-de-simulação--cfd-core-e-solvers)).

## 1. Visão geral e ordem

| Ordem | Método | Fluidos / fenómenos | Fase |
|-------|--------|---------------------|------|
| 1 | **LBM D2Q9** (BGK → TRT/MRT) | Gás/líquido monofásico incompressível (Ma baixo) | 1–2 |
| 2 | **LBM térmico D2Q5** (dupla distribuição) + Boussinesq | Convecção forçada e natural | 1–2 |
| 3 | **LBM superfície livre (VOF-LBM)** — no MVP | Líquidos com superfície livre (água a cair, ondas) | 6a |
| 4 | LBM multifásico (Shan-Chen / phase-field) | Bolhas, gotas, dois fluidos imiscíveis | 6 |
| 5 | Projeção (Chorin) em grelha MAC + multigrid | Incompressível "clássico"; comparação com LBM | 7 |
| 6 | Volumes finitos compressível (Godunov, HLLC, MUSCL) | Gases a alta velocidade, ondas de choque | 7 |
| 7 | SPH | Alternativa sem malha para líquidos | 10 |

## 2. Porque começar por LBM

- **Grelha cartesiana uniforme** = pixels. Fronteiras complexas desenhadas à mão resolvem-se com bounce-back sem geometria explícita.
- **Localidade total:** cada célula só lê os vizinhos imediatos → ideal para GPU (largura de banda é o único limite). Implementações de referência atingem milhares de MLUPS numa GPU.
- **Sem equação de Poisson global** no caso base (a pressão é `p = c_s² ρ`).
- Extensões bem estudadas para térmico, multifásico, superfície livre e turbulência (LES).

**Limitações** a ter presentes: só fracamente compressível (Ma ≲ 0.1–0.3), estabilidade sensível a τ próximo de 0.5 (Re de malha elevado), passo de tempo acoplado à resolução.

## 3. LBM D2Q9 — detalhes

### 3.1 Algoritmo por passo
1. **Colisão:** `f_i* = f_i − (f_i − f_i^eq)/τ + F_i` (BGK + forçamento de Guo para gravidade/flutuação).
2. **Streaming:** `f_i(x + c_i, t+1) = f_i*(x, t)`.
3. **Fronteiras:** bounce-back (paredes), Zou-He / velocidade imposta (inlets), pressão ou extrapolação (outlets).
4. **Macroscópicas:** `ρ = Σ f_i`, `ρu = Σ c_i f_i + F/2`.

Implementação em **um único kernel fundido** (stream-collide, "pull") com dois buffers (ping-pong). Otimização posterior: padrão **AA** ou **Esoteric Pull** para usar só um buffer (metade da memória).

### 3.2 Operadores de colisão (por robustez crescente)
1. BGK — simples, referência.
2. **TRT** (two-relaxation-time) — paredes independentes da viscosidade; boa relação custo/benefício. *Recomendado como default.*
3. MRT / regularizado / cumulantes — maior estabilidade a Re elevado.
4. LES Smagorinsky (τ efetivo local) para turbulência.

### 3.3 Conversão de unidades (no servidor)
Dados `dx` (= `cellSize`), velocidade física característica `U` e viscosidade `ν`:
- Escolher `u_lb` (velocidade em rede) ≤ 0.1 (default 0.05) → `dt = u_lb · dx / U`.
- `ν_lb = ν · dt / dx²` → `τ = 3 ν_lb + 0.5`.
- Verificações: `τ > 0.5 + ε` (ex.: 0.505 com TRT); `Ma = u_lb·√3 < 0.17`; aviso se `Re_malha = U·dx/ν` for elevado.
- Se a combinação for instável, o servidor **propõe** ao utilizador: aumentar a resolução, reduzir `u_lb` (mais passos) ou ativar LES.

### 3.4 Memória (f32)
`9 × 4 B × 2 buffers + flags + campos macroscópicos ≈ 85 B/célula`.
4096² ≈ 16.8 M células → ~1.4 GB. Com AA-pattern e f16 para armazenamento de `f_i` (técnica do FluidX3D) → ~0.5 GB.

## 4. Transporte de calor

- **Dupla distribuição:** segunda rede `g_i` D2Q5 para a equação de advecção-difusão da temperatura, com `τ_g = α_lb / c_s,g² + 0.5`, onde `c_s,g²` depende dos pesos D2Q5 escolhidos (ex.: `w₀ = 1/3`, `wᵢ = 1/6` ⇒ `c_s,g² = 1/3` ⇒ `τ_g = 3 α_lb + 0.5`). Documentar a escolha na implementação.
- **Acoplamento:** Boussinesq — força `F = −ρ₀ β (T − T_ref) g`, aplicada via forçamento de Guo.
- **Fronteiras térmicas:** temperatura fixa (anti-bounce-back), fluxo, adiabática (bounce-back de `g`).
- **Conjugado (sólido-fluido):** fase posterior — o sólido também resolve difusão com `α_s` próprio, com continuidade de fluxo na interface.

## 5. Líquidos e multifásico (Fase 6)

| Abordagem | Prós | Contras |
|-----------|------|---------|
| **LBM superfície livre** (Körner et al. 2005; massa por célula, células interface) | Reusa o kernel LBM, ótimo em GPU | Só uma fase (o gás é "vazio"), tensão superficial requer curvatura |
| **FLIP/APIC** (partículas + grelha MAC) | Visual excelente, pouco dissipativo | Requer Poisson (multigrid), partículas em GPU dão trabalho |
| **Shan-Chen** (pseudo-potencial LBM) | Simples, separação de fases espontânea | Correntes espúrias, rácios de densidade limitados |
| **Phase-field LBM** (Allen-Cahn conservativo) | Rácios de densidade elevados (água/ar ≈ 1000) | Mais complexo |

**Recomendação:** começar com LBM superfície livre (água no ar) e depois phase-field para dois fluidos reais.

## 6. Outros métodos (Fase 7+)

- **Projeção (Chorin/Stam) em grelha MAC:** advecção semi-Lagrangiana/BFECC ou MacCormack, Poisson por PCG com pré-condicionador multigrid; serve de base para FLIP e para comparação com LBM.
- **Volumes finitos compressível:** Euler/Navier-Stokes compressível, reconstrução MUSCL/WENO, fluxo HLLC, Runge-Kutta SSP; permite gases com Ma > 0.3 e choques.
- **Mudança de fase:** método de entalpia (solidificação/fusão), evaporação mais tarde.
- **Outros estados:** granular (DEM ou μ(I) contínuo), não-newtoniano (viscosidade dependente da taxa de corte — fácil em LBM com τ local).

## 7. Validação

Como medir cada caso, passo a passo: ver [08 — Guia de validação](08-guia-validacao.md). Cada solver tem de passar estes casos (automatizados em `validation/`, executados no CI no backend CPU com resolução reduzida e no GPU em nightly):

| Caso | Referência | Métrica alvo |
|------|------------|--------------|
| Poiseuille 2D | Solução analítica | Erro L2 do perfil < 1%; ordem de convergência ≈ 2 |
| Couette | Analítica | Erro L2 < 1% |
| Taylor-Green (decaimento) | Analítica | Taxa de decaimento da energia; ordem ≈ 2 |
| Cavidade com tampa móvel Re 100/400/1000 | Ghia, Ghia & Shin (1982) | Perfis u(y), v(x) no centro |
| Cilindro em canal Re 20 / Re 100 | Schäfer & Turek (1996) | Re 20: C_D ≈ 5.58; Re 100: St ≈ 0.30, C_D,max ≈ 3.23 |
| Cavidade aquecida diferencialmente Ra 10³–10⁶ | de Vahl Davis (1983) | Nu médio ≈ 1.118 / 2.243 / 4.519 / 8.800 |
| Rayleigh-Bénard | Teoria linear | Início da convecção em Ra_c ≈ 1708 |
| Dam break | Martin & Moyce (1952) | Posição da frente vs tempo |
| Tubo de choque de Sod | Solução exata | Perfis ρ, u, p (só compressível) |

**Paridade entre backends (CPU, wgpu, CUDA):** mesma cena, N passos, diferença relativa máxima < 1e-5 (f32) nos campos macroscópicos; o backend CPU é a referência. Para simulações longas/caóticas (ex.: esteira de von Kármán) compara-se estatística (St, C_D médio) em vez de campos instantâneos, porque pequenas diferenças de arredondamento divergem com o tempo. Ferramenta: `cfd-cli compare`.

## 8. Diagnóstico em execução

Calculado periodicamente (redução em GPU): massa total, energia cinética, velocidade máxima (alerta de Mach), deteção de NaN/Inf (aborta o job com mensagem útil e o último frame válido).

## 9. Referências principais

- Krüger et al., *The Lattice Boltzmann Method: Principles and Practice*, Springer, 2017.
- Guo, Zheng & Shi, forçamento em LBM, Phys. Rev. E 65, 2002.
- Ginzburg, modelos TRT, 2008.
- Körner et al., LBM com superfície livre, J. Comput. Phys., 2005.
- Lehmann, FluidX3D (otimizações de memória/f16 em LBM GPU), 2022.
- Bridson, *Fluid Simulation for Computer Graphics*, 2ª ed., 2015 (projeção, FLIP).
- Toro, *Riemann Solvers and Numerical Methods for Fluid Dynamics*, 2009 (compressível).
