import assert from 'node:assert/strict'
import { execFileSync, spawnSync } from 'node:child_process'
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { dirname, join, resolve } from 'node:path'
import test from 'node:test'
import { fileURLToPath } from 'node:url'

import { buildManifest, extractChangelogSection } from './make-update-manifest.cjs'

const script = resolve(dirname(fileURLToPath(import.meta.url)), 'make-update-manifest.cjs')

const CHANGELOG = `# Changelog

## [Unreleased]
Nothing yet.

## [1.9.0] - 2026-11-17

### Added

- Встроенное обновление.

## [1.8.0] - 2026-10-01

### Fixed

- Старое исправление.
`

function fixtureDir() {
  const dir = mkdtempSync(join(tmpdir(), 'ttsbard-manifest-test-'))
  writeFileSync(join(dir, 'CHANGELOG.md'), CHANGELOG, 'utf8')
  writeFileSync(join(dir, 'setup.exe'), 'fake installer bytes')
  writeFileSync(join(dir, 'setup.exe.sig'), 'dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHYx\n')
  return dir
}

test('extractChangelogSection returns exactly the tagged section', () => {
  const dir = fixtureDir()
  try {
    const notes = extractChangelogSection(join(dir, 'CHANGELOG.md'), '1.9.0')
    assert.equal(notes, '### Added\n\n- Встроенное обновление.')
  } finally {
    rmSync(dir, { recursive: true, force: true })
  }
})

test('extractChangelogSection does not bleed from a mismatched section', () => {
  const dir = fixtureDir()
  try {
    const changelog = join(dir, 'CHANGELOG.md')
    // The tag's own section is absent: neither a different version's section
    // nor the Unreleased block may leak into the notes.
    assert.equal(extractChangelogSection(changelog, '1.9.1'), '')
    assert.equal(extractChangelogSection(changelog, '1.9'), '')
    assert.equal(extractChangelogSection(changelog, '1.8.0'), '### Fixed\n\n- Старое исправление.')
    // A missing file is an empty section, not a failure.
    assert.equal(extractChangelogSection(join(dir, 'absent.md'), '1.9.0'), '')
  } finally {
    rmSync(dir, { recursive: true, force: true })
  }
})

test('--print-notes writes the section that the release body and the manifest share', () => {
  const dir = fixtureDir()
  try {
    const printed = spawnSync(
      process.execPath,
      [script, '--print-notes', '--version', '1.9.0', '--changelog', join(dir, 'CHANGELOG.md')],
      { encoding: 'utf8' },
    )
    assert.equal(printed.status, 0, printed.stderr)
    assert.equal(
      printed.stdout,
      extractChangelogSection(join(dir, 'CHANGELOG.md'), '1.9.0'),
      'the extraction mode prints the section byte-identically',
    )

    // The manifest built from that file carries the same notes, so the
    // release body and latest.json come from one source.
    const notesFile = join(dir, 'release-notes.md')
    writeFileSync(notesFile, printed.stdout, 'utf8')
    const out = join(dir, 'latest.json')
    execFileSync(process.execPath, [
      script,
      '--version', '1.9.0',
      '--setup', join(dir, 'setup.exe'),
      '--sig', join(dir, 'setup.exe.sig'),
      '--notes-file', notesFile,
      '--out', out,
    ])
    const manifest = JSON.parse(readFileSync(out, 'utf8'))
    assert.equal(manifest.version, '1.9.0')
    assert.equal(manifest.notes, printed.stdout.trim())
    assert.match(manifest.platforms['windows-x86_64'].url, /releases\/download\/1\.9\.0\/setup\.exe$/)
    assert.ok(manifest.platforms['windows-x86_64'].signature.trim().length > 0)
  } finally {
    rmSync(dir, { recursive: true, force: true })
  }
})

test('an unsigned installer is refused, mirroring the workflow no-signature path', () => {
  const dir = fixtureDir()
  try {
    writeFileSync(join(dir, 'empty.sig'), '   \n')
    const refused = spawnSync(
      process.execPath,
      [
        script,
        '--version', '1.9.0',
        '--setup', join(dir, 'setup.exe'),
        '--sig', join(dir, 'empty.sig'),
        '--out', join(dir, 'latest.json'),
      ],
      { encoding: 'utf8' },
    )
    assert.notEqual(refused.status, 0, 'an empty signature must fail the manifest build')
    assert.match(refused.stderr, /signature file is empty/)
    // Nothing was written for the refused build.
    assert.throws(() => readFileSync(join(dir, 'latest.json'), 'utf8'))
  } finally {
    rmSync(dir, { recursive: true, force: true })
  }
})

test('buildManifest keeps the fixed platform key and repo override', () => {
  const manifest = buildManifest({
    version: '2.0.0',
    setupPath: 'artifacts\\bundle\\nsis\\TTSBard_2.0.0_x64-setup.exe',
    signature: 'sig',
    notes: 'n',
    repo: 'acme/TTSBard',
  })
  assert.deepEqual(Object.keys(manifest.platforms), ['windows-x86_64'])
  assert.equal(
    manifest.platforms['windows-x86_64'].url,
    'https://github.com/acme/TTSBard/releases/download/2.0.0/TTSBard_2.0.0_x64-setup.exe',
  )
})
