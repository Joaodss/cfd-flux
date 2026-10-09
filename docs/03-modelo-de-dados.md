# 03 — Modelo de dados

## 1. Princípios

- **Separar geometria de propriedades:** os pixels guardam apenas *IDs* pequenos; as propriedades vivem numa tabela de elementos. Alterar a temperatura de uma parede não toca nos pixels.
- **Unidades físicas na cena, unidades de rede no solver.** A conversão é feita no servidor (`cfd-core::units`), nunca no frontend.
- **Versionado:** cada cena tem `schemaVersion`; migrações explícitas.
- **Independente do método:** a cena descreve o problema físico, não o algoritmo. O método é só um parâmetro.

## 2. Estrutura da cena (`Scene`)

```jsonc
{
  "schemaVersion": 1,
  "name": "Cilindro Re=100",
  "grid": { "width": 800, "height": 300, "cellSize": 0.001 },   // metros por pixel
  "fluids": [
    { "id": 1, "name": "Ar", "preset": "air",
      "density": 1.204, "kinematicViscosity": 1.5e-5,
      "thermalConductivity": 0.0257, "specificHeat": 1005, "thermalExpansion": 3.4e-3 }
  ],
  "solidMaterials": [
    { "id": 1, "name": "Alumínio", "thermalConductivity": 205, "density": 2700, "specificHeat": 900 }
  ],
  "elements": [
    { "id": 1, "kind": "wall",   "velocity": { "type": "noSlip" },
      "thermal": { "type": "adiabatic" }, "material": 1 },
    { "id": 2, "kind": "inlet",  "velocity": { "type": "uniform", "value": [0.15, 0.0] },
      "thermal": { "type": "fixed", "value": 293.15 }, "fluid": 1 },
    { "id": 3, "kind": "outlet", "pressure": { "type": "fixed", "value": 0.0 } },
    { "id": 4, "kind": "heatSource", "thermal": { "type": "fixed", "value": 350.0 }, "material": 1 }
  ],
  "probes": [ { "name": "Sonda A", "position": [600, 150] } ],   // coordenadas em células
  "initial": { "fluid": 1, "velocity": [0, 0], "temperature": 293.15, "pressure": 0 },
  "physics": { "gravity": [0, -9.81], "thermal": true, "buoyancy": "boussinesq" },
  "run": {
    "method": "lbm",                 // lbm | projection | flip | fv-compressible | surrogate
    "backend": "auto",               // auto | gpu | cpu
    "precision": "f32",
    "endTime": 2.0,                  // segundos físicos
    "outputInterval": 0.01,
    "outputFields": ["velocity", "pressure", "temperature", "vorticity"]
  },
  "layers": { "...": "ver secção 3" }
}
```

## 3. Camadas de pixels

Cada camada é uma matriz W×H (linha a linha, origem no canto inferior esquerdo para coincidir com a convenção física; o frontend inverte o eixo Y na renderização).

| Camada | Tipo | Significado |
|--------|------|-------------|
| `cellType` | `u8` | 0 = fluido, 1 = sólido, 2 = inlet, 3 = outlet, 4 = fonte de calor, 5 = vazio (gás "vazio" em superfície livre), … |
| `elementId` | `u16` | ID do elemento (tabela `elements`); 0 = nenhum |
| `fluidId` | `u8` | Fluido inicial na célula (multifásico); 0 = fluido por omissão |
| `probes` | lista esparsa (fora de `layers`) | Coordenadas das sondas (não precisa de matriz) |

> A implementação de referência do formato é `crates/cfd-core/src/scene.rs`; o schema gerado está em `schema/scene.schema.json`. Em caso de divergência com este documento, vale o código.

**Codificação no transporte:** cada camada é enviada como base64 de `zstd(raw bytes)` ou RLE simples (desenhos têm grandes áreas uniformes; compressão típica > 50×). Alternativa: PNG indexado (útil também para importar/exportar no editor).

```jsonc
"layers": {
  "cellType":  { "encoding": "zstd+base64", "dtype": "u8",  "data": "KLUv/..." },
  "elementId": { "encoding": "zstd+base64", "dtype": "u16", "data": "KLUv/..." }
}
```

Regras de consistência (validadas no cliente **e** no servidor):
- `cellType` de inlet/outlet/heatSource/sólido exige `elementId` válido do `kind` correspondente.
- Inlets e outlets devem estar adjacentes a pelo menos uma célula de fluido.
- Bordas do domínio sem marcação são tratadas como parede no-slip adiabática (configurável: periódica).

## 4. Tipos de condição de fronteira

### 4.1 Velocidade / pressão
| Tipo | Parâmetros | Implementação LBM (MVP) |
|------|------------|-------------------------|
| `noSlip` | — | Bounce-back (half-way) |
| `slip` | — | Specular reflection |
| `movingWall` | velocidade tangencial | Bounce-back com correção de momento |
| `uniform` (inlet) | vetor velocidade | Zou-He / bounce-back com velocidade |
| `profile` (inlet) | parabólico / tabela | Zou-He por célula |
| `massFlow` (inlet) | kg/s | Convertido em velocidade média |
| `pressure` (outlet) | pressão manométrica | Zou-He de pressão / anti-bounce-back |
| `zeroGradient` (outlet) | — | Extrapolação / convective outflow |
| `periodic` | par de bordas | Indexação periódica |

### 4.2 Térmica
| Tipo | Parâmetros |
|------|-----------|
| `adiabatic` | — (fluxo zero) |
| `fixed` | temperatura (K) |
| `flux` | W/m² |
| `convective` | h (W/m²K), T∞ |
| `volumetricSource` (no sólido) | W/m³ |

### 4.3 Futuras
Rugosidade/lei de parede (turbulência), ângulo de contacto (multifásico), porosidade (meios porosos), perfis dependentes do tempo (`value` pode ser `{ "type": "sine", "amp": ..., "freq": ... }` ou tabela tempo-valor).

## 5. Domínio interno (`Domain`, servidor)

Resultado da conversão `Scene → Domain` em `cfd-core`:
- Máscara de flags por célula (`u16` com bits: fluido, sólido, fronteira, tipo de BC, vizinho sólido em cada direção — pré-computado para kernels sem ramos).
- Tabelas de parâmetros por elemento já em **unidades de rede** (velocidade, densidade, τ, temperatura adimensional).
- Fatores de conversão (`dx`, `dt`, `ρ0`, `T_ref`, `ΔT`) e números adimensionais (Re, Ma, Pr, Ra) para relatório e verificação de estabilidade.

## 6. Resultados

### 6.1 Frame
```
FrameHeader { job_id, frame_index, sim_time, step, width, height, fields: [FieldDesc] }
FieldDesc   { name, components (1|2), dtype (f16|f32), scale/offset opcional, byte_len }
payload     = zstd( concatenação dos campos )
```
- Para visualização em streaming: **f16** e opcionalmente subamostragem (ex.: 2048² → 1024²).
- Para download/análise: f32 na resolução completa.

### 6.2 Armazenamento
```
results/{job_id}/
  scene.json          # cena exata usada (reprodutibilidade)
  meta.json           # versão do solver, backend, GPU, tempos, diagnósticos, Re/Ma/...
  frames/000123.bin   # frames codificados
  probes.csv          # séries temporais das sondas
  forces.csv          # arrasto/sustentação por elemento (se pedido)
```
Futuro: Zarr/HDF5 para leitura parcial eficiente; export VTK (`.vti`) para ParaView.

## 7. API (rascunho)

| Método | Rota | Descrição |
|--------|------|-----------|
| `POST` | `/api/simulations` | Cria job a partir de uma `Scene` → `{ id, estimatedMemory, warnings[] }` |
| `GET` | `/api/simulations/{id}` | Estado, progresso, diagnósticos |
| `DELETE` | `/api/simulations/{id}` | Cancela |
| `POST` | `/api/simulations/{id}/pause` · `/resume` | Controlo |
| `WS` | `/api/simulations/{id}/stream` | Mensagens: `progress` (JSON), `frame` (binário), `log`, `done` |
| `GET` | `/api/simulations/{id}/frames/{n}` | Frame individual |
| `GET` | `/api/simulations/{id}/probes.csv` | Sondas |
| `POST` | `/api/scenes/validate` | Valida sem executar (avisos de estabilidade) |
| `GET/POST` | `/api/scenes` | Guardar/listar cenas |
| `GET` | `/api/capabilities` | Métodos, backends e limites disponíveis |

Mensagens de controlo no WebSocket (futuro modo interativo): `{ "type": "patch", "rect": [...], "layer": "cellType", "data": ... }`, `{ "type": "setParam", ... }`.
