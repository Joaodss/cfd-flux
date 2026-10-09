// Generates TypeScript types for the scene format from the JSON Schema produced by Rust
// (`cargo run -p cfd-cli -- schema schema/scene.schema.json`). Run with `npm run gen:types`.
import { writeFile } from 'node:fs/promises'
import { fileURLToPath } from 'node:url'
import { compileFromFile } from 'json-schema-to-typescript'

const schemaPath = fileURLToPath(new URL('../../../schema/scene.schema.json', import.meta.url))
const outPath = fileURLToPath(new URL('../src/generated/scene.ts', import.meta.url))

const ts = await compileFromFile(schemaPath, {
    bannerComment:
        '/* Generated from schema/scene.schema.json by scripts/gen-types.mjs. Do not edit. */',
    additionalProperties: false,
    style: { semi: false, singleQuote: true, printWidth: 100 },
})
await writeFile(outPath, ts.replace(/\r\n/g, '\n'))
console.log(`wrote ${outPath}`)
