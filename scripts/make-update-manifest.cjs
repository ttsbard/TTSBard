#!/usr/bin/env node
/**
 * make-update-manifest.cjs — build the Tauri updater manifest (latest.json)
 * for a TTSBard release (ROADMAP-141, stage 2).
 *
 * The Tauri bundler signs the NSIS installer when
 * TAURI_SIGNING_PRIVATE_KEY is set and `bundle.createUpdaterArtifacts` is
 * enabled (scripts/build.ps1 -UpdaterArtifacts), producing a `.sig` file next
 * to the installer. This script combines the release version, the `.sig`
 * contents, the public download URL and the release notes (from the matching
 * CHANGELOG.md section, when present) into the manifest the updater plugin
 * fetches from the fixed release asset URL:
 *
 *   https://github.com/<repo>/releases/latest/download/latest.json
 *
 * The release body uses the same source: the workflow extracts the section
 * once with `--print-notes`, writes it to a file and feeds that file back via
 * `--notes-file`, so the GitHub release description and latest.json notes are
 * byte-identical and come from one CHANGELOG section per tag.
 *
 * Usage (release workflow):
 *   node scripts/make-update-manifest.cjs \
 *     --version 1.9.0 \
 *     --setup artifacts/app-windows-x86_64-x/bundle/nsis/TTSBard_1.9.0_x64-setup.exe \
 *     --sig    artifacts/app-windows-x86_64-x/bundle/nsis/TTSBard_1.9.0_x64-setup.exe.sig \
 *     --notes-file artifacts/release-notes.md \
 *     --out    artifacts/latest.json
 *
 * Standalone notes extraction (release body source):
 *   node scripts/make-update-manifest.cjs \
 *     --print-notes --version 1.9.0 --changelog CHANGELOG.md > notes.md
 *
 * No secrets are involved: the private key only ever signs inside the bundler.
 */

'use strict'

const { readFileSync, writeFileSync, statSync } = require('node:fs')
const path = require('node:path')

function parseArgs(argv) {
  const args = {}
  for (let i = 0; i < argv.length; i++) {
    const key = argv[i]
    if (!key.startsWith('--')) throw new Error(`Unexpected argument: ${key}`)
    const value =
      i + 1 < argv.length && !argv[i + 1].startsWith('--') ? argv[++i] : ''
    args[key.slice(2)] = value
  }
  return args
}

function fail(message) {
  console.error(`make-update-manifest: ${message}`)
  process.exit(1)
}

/**
 * Extract the CHANGELOG section for one version ("## [1.9.0] - date" up to
 * the next "## [" header). Returns an empty string when the version has no
 * section yet; the updater renders an empty notes block in that case.
 */
function extractChangelogSection(changelogPath, version) {
  let text
  try {
    text = readFileSync(changelogPath, 'utf8')
  } catch {
    return ''
  }
  const lines = text.split(/\r?\n/)
  const start = lines.findIndex((line) => line.startsWith('## [') && line.includes(`]`) && line.slice(4, line.indexOf(']')) === version)
  if (start === -1) return ''
  const section = []
  for (let i = start + 1; i < lines.length; i++) {
    if (lines[i].startsWith('## [')) break
    section.push(lines[i])
  }
  return section.join('\n').trim()
}

/**
 * The release notes for one version, resolved from the first available
 * source: an explicit `--notes` string, a `--notes-file` (the shared
 * extraction the workflow also uses for the release body), or the matching
 * CHANGELOG.md section directly.
 */
function resolveNotes(args) {
  if (args.notes !== undefined && args.notes !== '') return args.notes
  if (args['notes-file']) {
    try {
      return readFileSync(args['notes-file'], 'utf8').trim()
    } catch (error) {
      fail(`cannot read the notes file (${args['notes-file']}): ${error.message}`)
    }
  }
  return extractChangelogSection(args.changelog, args.version)
}

/**
 * Static format of the Tauri updater manifest; the platform key matches the
 * updater target lookup order (`windows-x86_64`, see updates/install.rs).
 */
function buildManifest({ version, setupPath, signature, notes, repo }) {
  const repository = repo || process.env.GITHUB_REPOSITORY || 'ttsbard/TTSBard'
  const fileName = path.posix.basename(setupPath.replace(/\\/g, '/'))
  return {
    version,
    notes,
    pub_date: new Date().toISOString().replace(/\.\d{3}Z$/, 'Z'),
    platforms: {
      'windows-x86_64': {
        signature,
        url: `https://github.com/${repository}/releases/download/v${version}/${fileName}`,
      },
    },
  }
}

function main() {
  const args = parseArgs(process.argv.slice(2))
  const version = args.version
  const setupPath = args.setup
  const sigPath = args.sig
  const outPath = args.out
  const changelogPath = args.changelog
  const repo = process.env.GITHUB_REPOSITORY || 'ttsbard/TTSBard'

  // Notes extraction mode: print the CHANGELOG section for the version so the
  // release workflow can feed the same bytes to the release body and, via
  // --notes-file, to latest.json.
  if (args['print-notes'] !== undefined) {
    if (!version || !changelogPath) {
      fail('usage: --print-notes --version <semver> --changelog CHANGELOG.md')
    }
    process.stdout.write(extractChangelogSection(changelogPath, version))
    return
  }

  if (!version || !setupPath || !sigPath || !outPath) {
    fail('usage: --version <semver> --setup <setup.exe> --sig <setup.exe.sig> --out latest.json [--notes-file notes.md | --notes text | --changelog CHANGELOG.md]')
  }

  let signature
  try {
    signature = readFileSync(sigPath, 'utf8')
  } catch (error) {
    fail(`cannot read the signature file (${sigPath}): ${error.message}`)
  }
  if (!signature.trim()) {
    fail(`the signature file is empty (${sigPath}); was the installer signed?`)
  }

  try {
    statSync(setupPath)
  } catch (error) {
    fail(`cannot stat the installer (${setupPath}): ${error.message}`)
  }

  const notes = resolveNotes(args)
  const manifest = buildManifest({ version, setupPath, signature, notes, repo })

  writeFileSync(outPath, `${JSON.stringify(manifest, null, 2)}\n`, 'utf8')
  console.log(`latest.json written: ${outPath}`)
  console.log(`  version:   ${version}`)
  console.log(`  url:       ${manifest.platforms['windows-x86_64'].url}`)
  console.log(`  notes:     ${notes ? `${notes.length} chars` : '(empty)'}`)
}

module.exports = { parseArgs, extractChangelogSection, resolveNotes, buildManifest }

if (require.main === module) {
  main()
}
