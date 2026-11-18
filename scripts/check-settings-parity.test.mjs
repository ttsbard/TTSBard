import { describe, it } from 'node:test'
import { strict as assert } from 'node:assert'
import { checkSettingsParity } from './check-settings-parity.mjs'

function runTest({ sourceText, populated, omitNull, rootExportName = 'AppSettingsDto' }) {
  return checkSettingsParity({
    populatedFixture: populated,
    omitNullFixture: omitNull,
    sourceText,
    rootExportName,
  })
}

describe('checkSettingsParity (production entry point)', () => {
  it('accepts an optional boolean emitted only for the non-default value', () => {
    const result = runTest({
      sourceText: 'export interface AppSettingsDto { enabled?: boolean }',
      populated: { enabled: false },
      omitNull: {},
    })
    assert.deepStrictEqual(result.errors, [])
  })

  it('rejects a boolean value when the optional TypeScript field is a string', () => {
    const result = runTest({
      sourceText: 'export interface AppSettingsDto { enabled?: string }',
      populated: { enabled: false },
      omitNull: {},
    })
    assert.ok(result.errors.some(error => error.includes('enabled') && error.includes('boolean')))
  })

  it('passes when fixtures match TypeScript types exactly', () => {
    const source = `
      export interface AppSettingsDto {
        name: string | null
        tags: string[]
        meta: Meta
      }
      interface Meta {
        version: number
      }
    `
    const populated = {
      name: 'test',
      tags: ['a', 'b'],
      meta: { version: 1 },
    }
    const omitNull = {
      name: null,
      tags: [],
      meta: { version: 0 },
    }

    const result = runTest({ sourceText: source, populated, omitNull })
    assert.deepStrictEqual(result.errors.length, 0, `expected 0 errors, got: ${result.errors.join('; ')}`)
  })

  it('detects Rust-only nested key', () => {
    const source = `
      export interface AppSettingsDto {
        meta: Meta
      }
      interface Meta {
        name: string
      }
    `
    const populated = { meta: { name: 'x', rust_extra: true } }
    const omitNull = { meta: { name: 'y' } }

    const result = runTest({ sourceText: source, populated, omitNull })
    assert.ok(
      result.errors.some(e => e.includes('rust_extra') && e.includes('not found in TypeScript side')),
      `expected Rust-only nested key error, got: ${result.errors.join('; ')}`
    )
  })

  it('detects TypeScript-only required nested key', () => {
    const source = `
      export interface AppSettingsDto {
        meta: Meta
      }
      interface Meta {
        name: string
        ts_only: string
      }
    `
    const populated = { meta: { name: 'x' } }
    const omitNull = { meta: { name: 'y' } }

    const result = runTest({ sourceText: source, populated, omitNull })
    assert.ok(
      result.errors.some(e => e.includes('ts_only') && e.includes('required in TypeScript but not present in either Rust fixture')),
      `expected TS-only required nested key error, got: ${result.errors.join('; ')}`
    )
  })

  it('detects optional/required mismatch: Rust requires but TS is optional', () => {
    const source = `
      export interface AppSettingsDto {
        name?: string
      }
    `
    const populated = { name: 'x' }
    const omitNull = { name: 'y' }

    const result = runTest({ sourceText: source, populated, omitNull })
    assert.ok(
      result.errors.some(e => e.includes('required in Rust') && e.includes('optional in TypeScript')),
      `expected optional/required mismatch (TS optional), got: ${result.errors.join('; ')}`
    )
  })

  it('detects optional/required mismatch: TS requires but Rust omits', () => {
    const source = `
      export interface AppSettingsDto {
        name: string
      }
    `
    const populated = { name: 'x' }
    const omitNull = {}

    const result = runTest({ sourceText: source, populated, omitNull })
    assert.ok(
      result.errors.some(e => e.includes('name') && e.includes('optional in Rust') && e.includes('required in TypeScript')),
      `expected optional/required mismatch (TS required), got: ${result.errors.join('; ')}`
    )
  })

  it('detects nullability mismatch', () => {
    const source = `
      export interface AppSettingsDto {
        name: string
      }
    `
    const populated = { name: null }
    const omitNull = { name: null }

    const result = runTest({ sourceText: source, populated, omitNull })
    assert.ok(
      result.errors.some(e => e.includes('null in Rust') && e.includes('non-nullable')),
      `expected nullability mismatch, got: ${result.errors.join('; ')}`
    )
  })

  it('detects primitive/category mismatch inside an array element', () => {
    const source = `
      export interface AppSettingsDto {
        items: Item[]
      }
      interface Item {
        age: number
      }
    `
    const populated = { items: [{ age: 'not-a-number' }] }
    const omitNull = { items: [] }

    const result = runTest({ sourceText: source, populated, omitNull })
    assert.ok(
      result.errors.some(e => e.includes('items[0].age') && e.includes('is string') && e.includes('expects a different category')),
      `expected array element category mismatch, got: ${result.errors.join('; ')}`
    )
  })

  it('detects primitive/category mismatch inside a record value', () => {
    const source = `
      export interface AppSettingsDto {
        map: Record<string, number>
      }
    `
    const populated = { map: { val: 'not-a-number' } }
    const omitNull = { map: {} }

    const result = runTest({ sourceText: source, populated, omitNull })
    assert.ok(
      result.errors.some(e => e.includes('map.val') && e.includes('is string') && e.includes('expects a different category')),
      `expected record value category mismatch, got: ${result.errors.join('; ')}`
    )
  })

  it('detects literal-union value outside the declared union', () => {
    const source = `
      export interface AppSettingsDto {
        theme: 'dark' | 'light'
      }
    `
    const populated = { theme: 'red' }
    const omitNull = { theme: 'dark' }

    const result = runTest({ sourceText: source, populated, omitNull })
    assert.ok(
      result.errors.some(e => e.includes('theme') && e.includes('not in the literal union')),
      `expected literal-union mismatch, got: ${result.errors.join('; ')}`
    )
  })

  it('detects missing/non-exported root AppSettingsDto', () => {
    const source = `
      interface AppSettingsDto {
        x: string
      }
    `
    const populated = { x: 'hello' }
    const omitNull = { x: 'world' }

    const result = runTest({ sourceText: source, populated, omitNull })
    assert.ok(
      result.errors.some(e => e.includes("AppSettingsDto") && e.includes('not found')),
      `expected missing export error, got: ${result.errors.join('; ')}`
    )
  })

  it('passes with nullable object types', () => {
    const source = `
      export interface AppSettingsDto {
        nested: Nested | null
      }
      interface Nested {
        val: string
      }
    `
    const populated = { nested: { val: 'hi' } }
    const omitNull = { nested: null }

    const result = runTest({ sourceText: source, populated, omitNull })
    assert.deepStrictEqual(result.errors.length, 0, `expected 0 errors, got: ${result.errors.join('; ')}`)
  })

  it('detects Rust-only top-level key', () => {
    const source = `
      export interface AppSettingsDto {
        name: string
      }
    `
    const populated = { name: 'x', rust_only: 42 }
    const omitNull = { name: 'y' }

    const result = runTest({ sourceText: source, populated, omitNull })
    assert.ok(
      result.errors.some(e => e.includes('rust_only') && e.includes('not found in TypeScript side')),
      `expected Rust-only top-level error, got: ${result.errors.join('; ')}`
    )
  })

  it('detects TS-only required top-level key', () => {
    const source = `
      export interface AppSettingsDto {
        name: string
        ts_only: string
      }
    `
    const populated = { name: 'x' }
    const omitNull = { name: 'y' }

    const result = runTest({ sourceText: source, populated, omitNull })
    assert.ok(
      result.errors.some(e => e.includes('ts_only') && e.includes('required in TypeScript') && e.includes('not present')),
      `expected TS-only top-level error, got: ${result.errors.join('; ')}`
    )
  })

  it('detects object/primitive category mismatch at top level', () => {
    const source = `
      export interface AppSettingsDto {
        nested: Nested
      }
      interface Nested {
        val: string
      }
    `
    const populated = { nested: 'not-an-object' }
    const omitNull = { nested: { val: 'x' } }

    const result = runTest({ sourceText: source, populated, omitNull })
    assert.ok(
      result.errors.some(e => e.includes('nested') && e.includes('is string') && e.includes('expects a different category')),
      `expected category mismatch at top level, got: ${result.errors.join('; ')}`
    )
  })

  it('detects nullable in Rust but non-nullable in TS at nested level', () => {
    const source = `
      export interface AppSettingsDto {
        meta: Meta
      }
      interface Meta {
        value: number
      }
    `
    const populated = { meta: { value: null } }
    const omitNull = { meta: { value: null } }

    const result = runTest({ sourceText: source, populated, omitNull })
    assert.ok(
      result.errors.some(e => e.includes('non-nullable in TypeScript')),
      `expected nested nullability mismatch, got: ${result.errors.join('; ')}`
    )
  })
})
