import { useEffect, useState } from 'react'
import type { Scene } from './generated/scene.ts'

const EXAMPLES = ['channel', 'cylinder-re100', 'heated-cavity-ra1e5', 'dam-break']

// Phase 0 placeholder: lists the example scenes. The pixel editor arrives in Phase 4.
export default function App() {
  const [scenes, setScenes] = useState<Scene[]>([])

  useEffect(() => {
    Promise.all(EXAMPLES.map((n) => import(`../../../scenes/${n}.json`))).then((mods) =>
      setScenes(mods.map((m) => m.default as Scene)),
    )
  }, [])

  return (
    <main>
      <h1>live-fluids</h1>
      <p>CFD simulator in the browser — under construction (Phase 0 done).</p>
      <h2>Example scenes</h2>
      <ul>
        {scenes.map((s) => (
          <li key={s.name}>
            <strong>{s.name}</strong> — {s.grid.width}×{s.grid.height} cells
            {s.description ? `: ${s.description}` : ''}
          </li>
        ))}
      </ul>
    </main>
  )
}
