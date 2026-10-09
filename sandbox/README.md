# sandbox — protótipos de aprendizagem

Código descartável para aprender Rust e GPU antes de escrever o solver a sério (Fase 0 do
[plano](../docs/05-plano-implementacao.md)). Cada protótipo é um projeto Cargo independente,
**fora** do workspace principal (`cargo new sandbox/<nome>`), por isso não entra no CI.

| # | Protótipo | Objetivo | Feito |
|---|-----------|----------|-------|
| 1 | `lbm-minimal` | LBM D2Q9 BGK num único `main.rs`: cavidade 128², escreve PNG da velocidade (crate `image`). Primeiro sequencial, depois `rayon` (`par_chunks_mut`). Medir MLUPS. | ⬜ |
| 2 | `wgpu-hello` | Somar dois vetores num compute shader WGSL e ler o resultado para a CPU. | ⬜ |
| 3 | `cuda-hello` | O mesmo com `cudarc` e um kernel `.cu` compilado por NVRTC (precisa do CUDA Toolkit). | ⬜ |
| 4 | `lbm-gpu` | Portar o kernel do protótipo 1 para wgpu **ou** CUDA e comparar MLUPS com a CPU. | ⬜ |

Dicas para quem vem de C#: ver a secção "Notas para quem vem de C#" no plano.
