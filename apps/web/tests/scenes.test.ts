import { readdirSync, readFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import { Ajv2020 } from 'ajv/dist/2020.js'
import { describe, expect, it } from 'vitest'
import type { Scene } from '../src/generated/scene.ts'

const repo = (rel: string) => fileURLToPath(new URL(`../../../${rel}`, import.meta.url))
const schema = JSON.parse(readFileSync(repo('schema/scene.schema.json'), 'utf8'))
const ajv = new Ajv2020({ allErrors: true, strict: false })
const validate = ajv.compile<Scene>(schema)

const sceneFiles = readdirSync(repo('scenes')).filter((f) => f.endsWith('.json'))

describe('example scenes', () => {
  it('exist', () => {
    expect(sceneFiles.length).toBeGreaterThan(0)
  })

  it.each(sceneFiles)('%s matches the JSON Schema', (file) => {
    const scene: unknown = JSON.parse(readFileSync(repo(`scenes/${file}`), 'utf8'))
    const ok = validate(scene)
    expect(validate.errors ?? []).toEqual([])
    expect(ok).toBe(true)
  })

  it('rejects unknown properties', () => {
    const scene = JSON.parse(readFileSync(repo(`scenes/${sceneFiles[0]}`), 'utf8'))
    scene.grid.colour = 'red'
    expect(validate(scene)).toBe(false)
  })
})
