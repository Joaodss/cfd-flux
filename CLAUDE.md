# CLAUDE.md

live-fluids — simulador CFD web: editor pixel a pixel no browser → servidor Rust local → solver em GPU (CUDA e wgpu) com fallback CPU. Estado: **Fase 0 concluída** (formato de cena, validação, frames, CLI, esqueleto web); ainda sem solver.

- Documentação em `docs/` (índice no `README.md`). Ler `docs/05-plano-implementacao.md` antes de começar qualquer fase; comandos e ambiente em `docs/desenvolvimento.md`.
- Língua: documentação de `docs/` em português (PT-PT); código, comentários e commits em inglês.
- Decisões e perguntas em aberto: `docs/07-decisoes-e-perguntas.md` — atualizar quando o autor responder ou uma decisão mudar.
- Formato da cena: fonte de verdade em `crates/cfd-core/src/scene.rs`. `schema/`, `scenes/` e `apps/web/src/generated/` são gerados (`cfd-cli schema`, `cfd-cli example --all scenes`, `npm run gen:types`); testes falham se estiverem desatualizados.
- Autor: muita experiência em CFD (não explicar física básica), pouca em Rust/GPU, vem de C# → explicar idiomas Rust/wgpu/CUDA quando relevantes, com paralelos a C#. Os protótipos em `sandbox/` são para o autor aprender: não os escrever por ele sem pedido.
- Ambiente local (Windows): toolchain `windows-gnu` não linka (`dlltool`); `cargo check/clippy/fmt` funcionam em Windows, testes correm no WSL Ubuntu com `CARGO_TARGET_DIR=$HOME/.cache/live-fluids-target` até o MSVC Build Tools ser instalado. GPU: RTX 4060 8 GB.
- Regras de trabalho:
  - Três backends: CPU (referência numérica), wgpu/WGSL e CUDA (`cfd-cuda`, feature `cuda`). Qualquer kernel GPU precisa de teste de paridade; manter a mesma disposição de memória nos três.
  - Todo o solver novo tem de passar os casos de validação de `docs/04-metodos-numericos.md` §7 (método em `docs/08-guia-validacao.md`).
  - Unidades físicas na cena; conversão para unidades de rede só no solver.
- Ao concluir tarefas do plano, marcar as checkboxes em `docs/05-plano-implementacao.md`.
