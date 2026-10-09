# 08 — Guia de validação: como medir a precisão

Este guia explica, passo a passo, como saber se o simulador está **certo** — e não apenas bonito. Complementa a tabela de casos em [04 §7](04-metodos-numericos.md#7-validação).

## 1. A ideia em três frases

1. Escolhe-se um problema cuja resposta **já é conhecida** (por uma fórmula exata ou por resultados publicados e aceites pela comunidade).
2. Simula-se esse problema e **mede-se a mesma grandeza** que a referência mede.
3. Compara-se com um **número** (erro) e um **limite** (tolerância). Se o erro passar o limite, o teste falha — automaticamente, no CI.

Há três tipos de referência, do mais fácil ao mais difícil de usar:

| Tipo | Exemplo | O que se compara |
|------|---------|------------------|
| **A. Solução analítica** (fórmula exata) | Poiseuille, Couette, Taylor-Green | O campo inteiro, ponto a ponto |
| **B. Dados tabelados de referência** | Cavidade (Ghia 1982), cavidade aquecida (de Vahl Davis 1983) | Valores em pontos/linhas específicos |
| **C. Quantidades integrais** | Cilindro (Schäfer & Turek 1996), dam break | Um ou poucos números: C_D, St, Nu, posição da frente |

## 2. As medidas de erro

Seja `s` o valor simulado e `r` o valor de referência, em `N` pontos:

- **Erro L2 relativo** (o principal):
  `E_L2 = sqrt( Σ (s − r)² ) / sqrt( Σ r² )` → um número; 0.01 = 1% de erro.
- **Erro máximo** (L∞): `max |s − r|` — apanha erros localizados (ex.: junto a paredes).
- **Erro relativo de uma grandeza**: `|s − r| / |r|` — para C_D, St, Nu.

## 3. Ordem de convergência (o teste mais importante)

Um erro pequeno numa só resolução pode ser sorte. A prova forte é: **quando a grelha fica mais fina, o erro desce à taxa esperada.**

1. Corre o mesmo caso com N = 32, 64, 128, 256 células na direção característica.
2. Calcula o erro L2 em cada uma.
3. A ordem é `p = log(E_N / E_2N) / log(2)`.
4. LBM é um método de **2ª ordem**: espera-se `p ≈ 2` (erro cai para ~1/4 quando se duplica a resolução).

Num gráfico log-log (erro vs. resolução) os pontos devem formar uma reta com inclinação −2. Se `p ≈ 1`, há quase sempre um bug nas condições de fronteira.

> **Nota LBM:** ao refinar, mantém-se o número de Reynolds e usa-se *escalamento difusivo* (`u_lb ∝ 1/N`), caso contrário o erro de compressibilidade (Mach) não diminui e a ordem aparente fica errada.

## 4. Os casos, explicados

### 4.1 Poiseuille (escoamento num canal) — tipo A
- **Montagem:** canal entre duas paredes paralelas, escoamento empurrado por uma força constante (ou diferença de pressão), fronteiras periódicas na direção do escoamento.
- **Referência:** perfil parabólico `u(y) = (G / 2ν) · y · (H − y)`.
- **Medir:** quando o escoamento estabiliza (variação entre passos < 1e-10), comparar `u(y)` numa coluna com a fórmula → E_L2. Repetir em 4 resoluções → ordem.
- **Alvo:** E_L2 < 1% a N = 32; p ≈ 2.

### 4.2 Taylor-Green (vórtices a decair) — tipo A
- **Montagem:** domínio periódico com um padrão de vórtices inicial conhecido, sem paredes.
- **Referência:** a amplitude decai como `exp(−2 ν k² t)`.
- **Medir:** energia cinética total ao longo do tempo vs. fórmula; campo de velocidade num instante fixo → E_L2 e ordem. É o melhor teste do **núcleo** do solver (sem fronteiras).

### 4.3 Cavidade com tampa móvel — tipo B
- **Montagem:** quadrado fechado, parede de cima a mover-se com velocidade U. Re = U·L/ν = 100, 400, 1000.
- **Referência:** tabelas de Ghia et al. (1982): `u` ao longo da linha vertical central e `v` ao longo da horizontal central (17 pontos cada).
- **Medir:** em regime estacionário, interpolar o campo simulado nesses pontos → erro máximo e L2.
- **Alvo:** desvio < 1–2% da velocidade da tampa.

### 4.4 Cilindro num canal (Schäfer & Turek 2D-1 e 2D-2) — tipo C
- **Montagem:** canal 2.2 m × 0.41 m, cilindro de diâmetro 0.1 m ligeiramente descentrado, perfil parabólico à entrada.
- **Medir:**
  - **Força no cilindro** (método *momentum exchange* em LBM) → coeficientes `C_D = 2F_x / (ρ U² D)`, `C_L` idem com `F_y`.
  - **Re 20** (estacionário): C_D final. Alvo: 5.57–5.59.
  - **Re 100** (periódico): série temporal de C_L → frequência dominante `f` por FFT ou contagem de zeros → **Strouhal** `St = f·D/U`. Alvo: 0.295–0.305; C_D máximo 3.22–3.24.
- **Atenção:** um cilindro desenhado em pixels tem "escadas"; precisa de ~20+ células no diâmetro para ficar dentro das tolerâncias. Bom caso para comparar resoluções e, mais tarde, o *interpolated bounce-back*.

### 4.5 Cavidade aquecida diferencialmente (de Vahl Davis 1983) — tipo B/C
- **Montagem:** quadrado fechado, parede esquerda quente, direita fria, topo/fundo adiabáticos, gravidade. Número de Rayleigh Ra = 10³ … 10⁶.
- **Medir:** **número de Nusselt médio** na parede quente: `Nu = (L / ΔT) · média de (−∂T/∂x)` na parede. Também velocidades máximas nas linhas centrais.
- **Alvo:** Nu ≈ 1.118 / 2.243 / 4.519 / 8.800 (Ra 10³ / 10⁴ / 10⁵ / 10⁶), erro < 1–2%.

### 4.6 Dam break (Martin & Moyce 1952) — tipo C, líquidos
- **Montagem:** coluna de água de largura `a` encostada a uma parede, liberta no instante 0.
- **Medir:** posição da frente de água `x(t)` em tempo adimensional `t·sqrt(2g/a)`, comparada com os pontos experimentais. Também a **conservação de massa** do líquido (deve ficar < 0.1%).
- **Alvo:** curva dentro da dispersão experimental (é dado experimental, não exato — tolerância maior, ~5%).

## 5. Verificações que não precisam de referência

Correm em **todas** as simulações, não só nos testes:
- Massa total constante (sistemas fechados) — deriva < 1e-6 relativo.
- Sem NaN/Inf.
- Simetria: um caso simétrico deve dar resultado simétrico (até a instabilidade física a quebrar).
- Mach máximo < ~0.17 (limite de validade do LBM).

## 6. Como fica no código

```
validation/
  references/            # dados de referência em CSV (Ghia, de Vahl Davis, Martin-Moyce, ...) com fonte citada
  cases/*.json           # cenas de validação (formato Scene normal)
  run_validation.rs      # (em cfd-cli: `cfd-cli validate`) corre casos, calcula erros, compara com tolerâncias
  report/                # gerado: tabela Markdown + gráficos (perfis, convergência log-log, séries C_L)
```

- `cfd-cli validate --backend cpu --quick` → resoluções baixas, corre no CI em cada commit (minutos).
- `cfd-cli validate --backend cuda --full` → todas as resoluções; corre localmente antes de releases.
- O relatório é publicado na documentação: **é um ótimo elemento de portfólio** ("este simulador reproduz Ghia 1982 com erro < 1%").

## 7. Ordem recomendada

1. Taylor-Green (testa só o núcleo) → 2. Poiseuille (testa paredes e forçamento) → 3. Cavidade (paredes móveis) → 4. Cilindro (inlet/outlet, forças) → 5. Cavidade aquecida (térmico) → 6. Dam break (superfície livre).

Cada passo isola uma parte nova do código: se falhar, sabe-se onde procurar.
