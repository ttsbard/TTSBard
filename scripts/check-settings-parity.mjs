import { readFileSync, existsSync } from 'node:fs'
import { resolve, dirname } from 'node:path'
import { fileURLToPath, pathToFileURL } from 'node:url'
import ts from 'typescript'

const __root = resolve(dirname(fileURLToPath(import.meta.url)), '..')
const SETTINGS_TS = resolve(__root, 'src/types/settings.ts')
const TYPES_TS = resolve(__root, 'src/types.ts')
const FIXTURE_DIR = resolve(__root, 'scripts/settings-fixtures')

function jsonCategory(value) {
  if (value === null) return 'null'
  if (Array.isArray(value)) return 'array'
  if (typeof value === 'object') return 'object'
  if (typeof value === 'string') return 'string'
  if (typeof value === 'number') return 'number'
  if (typeof value === 'boolean') return 'boolean'
  return 'unknown'
}

export { jsonCategory }

function allowsNull(tsDesc) {
  return tsDesc.kind === 'null' || tsDesc.nullable || tsDesc.hasNull
}

export { allowsNull }

function matchesCategory(tsDesc, cat) {
  if (cat === 'null') return allowsNull(tsDesc)
  const inner = tsDesc.nullable ? { ...tsDesc, nullable: false } : tsDesc

  switch (cat) {
    case 'string':
      return (inner.kind === 'primitive' && inner.tsPrimitive === 'string')
        || (inner.kind === 'literal' && inner.literalType === 'string')
        || (inner.kind === 'literal-union' && inner.literals.length > 0 && inner.literals.every(l => l.literalType === 'string'))
    case 'number':
      return (inner.kind === 'primitive' && inner.tsPrimitive === 'number')
        || (inner.kind === 'literal' && inner.literalType === 'number')
        || (inner.kind === 'literal-union' && inner.literals.length > 0 && inner.literals.every(l => l.literalType === 'number'))
    case 'boolean':
      return (inner.kind === 'primitive' && inner.tsPrimitive === 'boolean')
        || (inner.kind === 'literal' && inner.literalType === 'boolean')
        || (inner.kind === 'literal-union' && inner.literals.length > 0 && inner.literals.every(l => l.literalType === 'boolean'))
    case 'array':
      return inner.kind === 'array'
    case 'object':
      return inner.kind === 'object' || inner.kind === 'record'
    default:
      return false
  }
}

// ---------------------------------------------------------------------------
// TS compiler-backed type description
// ---------------------------------------------------------------------------

function createDiskProgram(sourcePaths) {
  return ts.createProgram(sourcePaths, {
    target: ts.ScriptTarget.ESNext,
    module: ts.ModuleKind.ESNext,
    strict: true,
  })
}

export { createDiskProgram }

function createSyntheticProgram(sourceText, additionalPaths) {
  const fileName = '/_synthetic.ts'
  const options = {
    strict: true,
    target: ts.ScriptTarget.ESNext,
    module: ts.ModuleKind.ESNext,
  }
  const host = ts.createCompilerHost(options, true)
  const baseGetSourceFile = host.getSourceFile.bind(host)
  const baseFileExists = host.fileExists.bind(host)
  const baseReadFile = host.readFile.bind(host)

  host.getSourceFile = (requested, languageVersion, onError, shouldCreateNewSourceFile) => {
    if (requested === fileName) {
      return ts.createSourceFile(fileName, sourceText, languageVersion, true, ts.ScriptKind.TS)
    }
    return baseGetSourceFile(requested, languageVersion, onError, shouldCreateNewSourceFile)
  }
  host.fileExists = requested => requested === fileName || baseFileExists(requested)
  host.readFile = requested => requested === fileName ? sourceText : baseReadFile(requested)

  return ts.createProgram([fileName, ...(additionalPaths || []).map(resolve)], options, host)
}

export { createSyntheticProgram }

function findExportedType(program, sourceFileName, exportName) {
  const checker = program.getTypeChecker()
  const sourceFile = program.getSourceFile(sourceFileName)
  if (!sourceFile) return null
  const moduleSymbol = checker.getSymbolAtLocation(sourceFile)
  if (!moduleSymbol) return null
  const exported = checker.getExportsOfModule(moduleSymbol).find(symbol => symbol.getName() === exportName)
  if (!exported) return null
  const declarations = exported.getDeclarations()
  if (!declarations || declarations.length === 0) return null
  return checker.getTypeAtLocation(declarations[0])
}

export { findExportedType }

function describeType(checker, type, visited, symbolName) {
  const SF = ts.TypeFlags
  const flags = type.flags

  if (flags & SF.Any || flags & SF.Unknown) return { kind: 'primitive', tsPrimitive: 'any' }
  if (flags & SF.Void) return { kind: 'void' }
  if (flags & SF.Null) return { kind: 'null' }
  if (flags & SF.Undefined) return { kind: 'undefined' }
  if (flags & SF.Never) return { kind: 'never' }

  if (flags & SF.String) return { kind: 'primitive', tsPrimitive: 'string' }
  if (flags & SF.Number) return { kind: 'primitive', tsPrimitive: 'number' }
  if (flags & SF.Boolean) return { kind: 'primitive', tsPrimitive: 'boolean' }

  if (flags & SF.StringLiteral) return { kind: 'literal', literalType: 'string', value: type.value }
  if (flags & SF.NumberLiteral) return { kind: 'literal', literalType: 'number', value: type.value }
  if (flags & SF.BooleanLiteral) return { kind: 'literal', literalType: 'boolean', value: checker.typeToString(type) === 'true' }

  if (flags & SF.EnumLiteral) {
    const str = checker.typeToString(type)
    if (typeof str === 'string') return { kind: 'literal', literalType: 'string', value: str.replace(/^['"]|['"]$/g, '') }
    return { kind: 'unknown', name: str }
  }

  if (type.isUnion()) {
    return describeUnion(checker, type, visited)
  }

  if (type.isIntersection()) {
    const parts = type.types.map(t => describeType(checker, t, visited))
    return parts.find(p => p.kind === 'object' && p.properties.size > 0)
      || parts.find(p => p.kind !== 'unknown')
      || { kind: 'unknown', name: checker.typeToString(type) }
  }

  if (flags & SF.Object) {
    return describeObject(checker, type, visited, symbolName)
  }

  return { kind: 'unknown', name: checker.typeToString(type) }
}

export { describeType }

function describeUnion(checker, unionType, visited) {
  const parts = []
  let hasNull = false
  let hasUndefined = false
  let skipped = false

  for (const t of unionType.types) {
    const d = describeType(checker, t, visited)
    if (d.kind === 'null') { hasNull = true }
    else if (d.kind === 'undefined') { hasUndefined = true }
    else if (d.kind === 'unknown' || d.kind === 'union') {
      if (d.kind === 'union') {
        parts.push(...d.types)
        if (d.hasNull) hasNull = true
        if (d.hasUndefined) hasUndefined = true
      } else {
        skipped = true
        parts.push(d)
      }
    } else {
      parts.push(d)
    }
  }

  // A TypeScript optional property includes undefined, but that must never make
  // an explicit JSON null acceptable. Preserve the two semantics separately.
  if ((hasNull || hasUndefined) && parts.length === 1 && parts[0].kind !== 'unknown') {
    return { ...parts[0], nullable: hasNull, hasUndefined }
  }

  // literal-union: all non-void parts are literals
  const nonVoid = parts
  if (nonVoid.length > 0 && nonVoid.every(p => p.kind === 'literal')) {
    return { kind: 'literal-union', literals: nonVoid.map(p => ({ literalType: p.literalType, value: p.value })), hasNull, hasUndefined }
  }

  if (skipped) {
    const nonSkipped = nonVoid.filter(p => p.kind !== 'unknown')
    if ((hasNull || hasUndefined) && nonSkipped.length === 1 && nonSkipped[0].kind !== 'unknown') {
      return { ...nonSkipped[0], nullable: hasNull, hasUndefined }
    }
    return { kind: 'unknown', name: checker.typeToString(unionType) }
  }

  return { kind: 'union', types: nonVoid, hasNull, hasUndefined }
}

function describeObject(checker, objType, visited, symbolName) {
  const typeId = objType.id
  if (visited && visited.has(typeId)) {
    return { kind: 'object', properties: new Map(), _circular: true }
  }
  const nestedVisited = visited ? new Set(visited) : new Set()
  nestedVisited.add(typeId)

  // Ask the checker first so aliases and both Array<T>/T[] spellings work.
  if (checker.isArrayType(objType) || checker.isTupleType(objType)) {
    const args = checker.getTypeArguments(objType)
    const elementType = args.length === 1
      ? args[0]
      : checker.getIndexTypeOfType(objType, ts.IndexKind.Number)
    return { kind: 'array', elementType: elementType ? describeType(checker, elementType, nestedVisited) : { kind: 'unknown', name: 'any' } }
  }

  // Array detection via symbol: fallback for unusual compiler types.
  const sym = objType.symbol
  if (sym && sym.name === 'Array') {
    const args = objType.typeArguments || []
    if (args.length > 0) {
      return { kind: 'array', elementType: describeType(checker, args[0], nestedVisited) }
    }
    // Fallback: number index
    const numIdx = objType.getNumberIndexType()
    if (numIdx) {
      return { kind: 'array', elementType: describeType(checker, numIdx, nestedVisited) }
    }
    return { kind: 'array', elementType: { kind: 'unknown', name: 'any' } }
  }

  // Record detection via symbol
  if (sym && sym.name === 'Record') {
    const args = objType.typeArguments || []
    if (args.length >= 2) {
      return { kind: 'record', valueType: describeType(checker, args[1], nestedVisited) }
    }
    // Fallback: string index
    const strIdx = objType.getStringIndexType()
    if (strIdx) {
      return { kind: 'record', valueType: describeType(checker, strIdx, nestedVisited) }
    }
    return { kind: 'record', valueType: { kind: 'unknown', name: 'any' } }
  }

  // Generic array-like: number index type
  const numIndex = objType.getNumberIndexType()
  if (numIndex) {
    return { kind: 'array', elementType: describeType(checker, numIndex, nestedVisited) }
  }

  // Record-like: string index type
  const strIndex = objType.getStringIndexType()
  const properties = checker.getPropertiesOfType(objType)

  if (strIndex) {
    const valueType = describeType(checker, strIndex, nestedVisited)

    const props = new Map()
    for (const prop of properties) {
      const name = prop.getName()
      if (name === '__index' || name === '__stringIndex' || name.startsWith('__')) continue
      if (name === 'toString' || name === 'toLocaleString' || name === 'valueOf'
        || name === 'hasOwnProperty' || name === 'isPrototypeOf' || name === 'propertyIsEnumerable'
        || name === 'constructor' || name === '__proto__') continue

      const decls = prop.getDeclarations()
      if (decls && decls.length > 0) {
        const decl = decls[0]
        if (ts.isIndexSignatureDeclaration(decl)) continue
      }

      const isOptional = (prop.getFlags() & ts.SymbolFlags.Optional) !== 0
      const propType = checker.getTypeOfSymbol(prop)
      props.set(name, { optional: isOptional, type: describeType(checker, propType, nestedVisited, name) })
    }

    if (props.size > 0) {
      return { kind: 'object', properties: props, recordOf: valueType }
    }
    return { kind: 'record', valueType }
  }

  // Plain object / interface
  const props = new Map()
  for (const prop of properties) {
    const name = prop.getName()
    if (name.startsWith('__')) continue
    if (name === 'toString' || name === 'toLocaleString' || name === 'valueOf'
      || name === 'hasOwnProperty' || name === 'isPrototypeOf' || name === 'propertyIsEnumerable'
      || name === 'constructor' || name === '__proto__') continue

    const decls = prop.getDeclarations()
    if (decls && decls.length > 0) {
      const decl = decls[0]
      if (ts.isIndexSignatureDeclaration(decl)) continue
      if (ts.isMethodSignature(decl)) continue
    }

    const isOptional = (prop.getFlags() & ts.SymbolFlags.Optional) !== 0
    const propType = checker.getTypeOfSymbol(prop)
    props.set(name, { optional: isOptional, type: describeType(checker, propType, nestedVisited, name) })
  }

  return { kind: 'object', properties: props }
}

// ---------------------------------------------------------------------------
// Fixture-based Rust semantics inference
// ---------------------------------------------------------------------------

function inferSemantics(popVal, omitVal) {
  const inPop = popVal !== undefined
  const inOmit = omitVal !== undefined

  return {
    required: inPop && inOmit,         // present in both = always serialized
    optional: inPop && !inOmit,         // only in populated = skip_serializing_if
    nullable: (inPop && popVal === null) || (inOmit && omitVal === null),
  }
}

// ---------------------------------------------------------------------------
// Recursive comparison
// ---------------------------------------------------------------------------

function compareValue(errors, popVal, omitVal, tsDesc, path) {
  const popCat = jsonCategory(popVal)
  const omitCat = jsonCategory(omitVal)

  // Check nullability
  if (popCat === 'null' || omitCat === 'null') {
    if (!allowsNull(tsDesc)) {
      errors.push(`[TS] key '${path}' is null in Rust fixture but TypeScript declares it non-nullable (tsKind=${tsDesc.kind})`)
    }
  }

  // Check categories and literal values from both fixtures. A mismatch in the
  // omit/null fixture must not be hidden by a valid populated value.
  const concreteValues = [popVal, omitVal].filter(value => value !== undefined && value !== null)
  for (const checkVal of concreteValues) {
    const cat = jsonCategory(checkVal)
    if (!matchesCategory(tsDesc, cat)) {
      errors.push(`[TS] key '${path}' is ${cat} in Rust fixture but TypeScript expects a different category (tsKind=${tsDesc.kind})`)
      return
    }
    if (tsDesc.kind === 'literal-union') {
      const literalValues = tsDesc.literals.map(literal => literal.value)
      if (!literalValues.includes(checkVal)) {
        errors.push(`[TS] key '${path}' has value '${JSON.stringify(checkVal)}' which is not in the literal union (allowed: ${literalValues.map(JSON.stringify).join(', ')})`)
        return
      }
    } else if (tsDesc.kind === 'literal' && tsDesc.value !== checkVal) {
      errors.push(`[TS] key '${path}' has value '${JSON.stringify(checkVal)}' but TypeScript requires literal ${JSON.stringify(tsDesc.value)}`)
      return
    }
  }

  // Recurse into objects
  if (tsDesc.kind === 'object') {
    const popObj = (popCat === 'object') ? popVal : null
    const omitObj = (omitCat === 'object') ? omitVal : null
    if (popObj && omitObj) {
      compareObjectPair(errors, popObj, omitObj, tsDesc, path)
    } else if (popObj) {
      compareObjectSingle(errors, popObj, tsDesc, path, 'populated')
    } else if (omitObj) {
      compareObjectSingle(errors, omitObj, tsDesc, path, 'omit-null')
    }
    return
  }

  // Recurse into arrays
  if (tsDesc.kind === 'array') {
    const elemDesc = tsDesc.elementType
    if (!elemDesc) return

    const popArr = Array.isArray(popVal) ? popVal : []
    const omitArr = Array.isArray(omitVal) ? omitVal : []

    const arrayLength = Math.max(popArr.length, omitArr.length)
    for (let i = 0; i < arrayLength; i++) {
      const elemPath = `${path}[${i}]`
      const popItem = i < popArr.length ? popArr[i] : undefined
      const omitItem = i < omitArr.length ? omitArr[i] : undefined
      compareValue(errors, popItem, omitItem, elemDesc, elemPath)
    }
    return
  }

  // Recurse into records
  if (tsDesc.kind === 'record') {
    const popObj = popCat === 'object' ? popVal : {}
    const omitObj = omitCat === 'object' ? omitVal : {}
    const allKeys = new Set([...Object.keys(popObj), ...Object.keys(omitObj)])
    for (const key of allKeys) {
      const recPath = `${path}.${key}`
      compareValue(errors, popObj[key], omitObj[key], tsDesc.valueType, recPath)
    }
    return
  }

}

function compareObjectPair(errors, populated, omitNull, tsDesc, path) {
  const tsProps = tsDesc.properties || new Map()
  const recordOf = tsDesc.recordOf || null
  const allKeys = new Set([...Object.keys(populated), ...Object.keys(omitNull)])

  for (const key of allKeys) {
    const jsonPath = path ? `${path}.${key}` : key
    const tsProp = tsProps.get(key)
    const popVal = populated[key]
    const omitVal = omitNull[key]
    const inPop = popVal !== undefined
    const inOmit = omitVal !== undefined

    if (!tsProp) {
      if (recordOf) {
        if (inPop && popVal !== null && jsonCategory(popVal) !== 'null') {
          compareValue(errors, popVal, omitVal, recordOf, jsonPath)
        } else if (inOmit && omitVal !== null && jsonCategory(omitVal) !== 'null') {
          compareValue(errors, popVal, omitVal, recordOf, jsonPath)
        }
      } else {
        const val = inPop ? popVal : omitVal
        errors.push(`[Rust] key '${jsonPath}' (${jsonCategory(val)}) not found in TypeScript side`)
      }
      continue
    }

    // Check category and value
    compareValue(errors, popVal, omitVal, tsProp.type, jsonPath)

    // Check optional/required/nullable from inferred semantics
    const sem = inferSemantics(popVal, omitVal)

    if (sem.required && tsProp.optional) {
      errors.push(`[TS] key '${jsonPath}' is required in Rust (present in both fixtures) but optional in TypeScript`)
    }
    if (sem.optional && !tsProp.optional) {
      errors.push(`[TS] key '${jsonPath}' is optional in Rust (only in populated fixture) but required in TypeScript`)
    }
    if (sem.nullable && !allowsNull(tsProp.type)) {
      errors.push(`[TS] key '${jsonPath}' is nullable in Rust but non-nullable in TypeScript`)
    }
  }

  // TS-only keys
  for (const [key, tsProp] of tsProps) {
    if (key in populated || key in omitNull) continue
    if (!tsProp.optional) {
      const jsonPath = path ? `${path}.${key}` : key
      errors.push(`[TS] key '${jsonPath}' is required in TypeScript but not present in either Rust fixture`)
    }
  }
}

function compareObjectSingle(errors, fixture, tsDesc, path, side) {
  const tsProps = tsDesc.properties || new Map()
  const recordOf = tsDesc.recordOf || null

  for (const key of Object.keys(fixture)) {
    const jsonPath = path ? `${path}.${key}` : key
    const tsProp = tsProps.get(key)
    const val = fixture[key]

    if (!tsProp) {
      if (recordOf && val !== null && jsonCategory(val) !== 'null') {
        compareValue(errors, val, undefined, recordOf, jsonPath)
      } else {
        errors.push(`[Rust] key '${jsonPath}' (${jsonCategory(val)}) not found in TypeScript side`)
      }
      continue
    }

    compareValue(errors, val, undefined, tsProp.type, jsonPath)
  }

  for (const [key, tsProp] of tsProps) {
    if (key in fixture) continue
    if (!tsProp.optional) {
      const jsonPath = path ? `${path}.${key}` : key
      errors.push(`[TS] key '${jsonPath}' is required in TypeScript but not in ${side} Rust fixture`)
    }
  }
}

// ---------------------------------------------------------------------------
// Main production entry point
// ---------------------------------------------------------------------------

export function checkSettingsParity({
  populatedFixture,
  omitNullFixture,
  rootExportName = 'AppSettingsDto',
  sourcePath,
  sourceText,
  additionalSourcePaths = [],
}) {
  if (!populatedFixture || !omitNullFixture) {
    return { errors: ['Both populatedFixture and omitNullFixture are required'] }
  }

  let program
  let rootSourceFileName
  if (sourceText !== undefined) {
    program = createSyntheticProgram(sourceText, additionalSourcePaths)
    rootSourceFileName = '/_synthetic.ts'
  } else if (sourcePath) {
    rootSourceFileName = resolve(sourcePath)
    const paths = [rootSourceFileName, ...additionalSourcePaths]
    program = createDiskProgram(paths)
  } else {
    program = createDiskProgram([SETTINGS_TS, TYPES_TS])
    rootSourceFileName = SETTINGS_TS
  }

  const rootType = findExportedType(program, rootSourceFileName, rootExportName)
  if (!rootType) {
    return { errors: [`export '${rootExportName}' not found in TypeScript source`] }
  }

  const checker = program.getTypeChecker()
  const rootDesc = describeType(checker, rootType, new Set())

  if (rootDesc.kind !== 'object' && rootDesc.kind !== 'record') {
    return { errors: [`export '${rootExportName}' is not an object type (kind=${rootDesc.kind})`] }
  }

  const errors = []
  compareObjectPair(errors, populatedFixture, omitNullFixture, rootDesc, '')
  return { errors }
}

// ---------------------------------------------------------------------------
// CLI wrapper
// ---------------------------------------------------------------------------

const isCli = process.argv[1]
  && import.meta.url === pathToFileURL(resolve(process.argv[1])).href

if (isCli) {
  if (!existsSync(resolve(FIXTURE_DIR, 'populated.json')) || !existsSync(resolve(FIXTURE_DIR, 'omit-null.json'))) {
    console.error('Fixtures missing in scripts/settings-fixtures/ — run `cargo test app_settings_dto_fixtures_regenerate -- --ignored` first')
    process.exitCode = 1
  } else {
    const populated = JSON.parse(readFileSync(resolve(FIXTURE_DIR, 'populated.json'), 'utf8'))
    const omitNull = JSON.parse(readFileSync(resolve(FIXTURE_DIR, 'omit-null.json'), 'utf8'))

    const result = checkSettingsParity({
      populatedFixture: populated,
      omitNullFixture: omitNull,
      sourcePath: SETTINGS_TS,
      additionalSourcePaths: existsSync(TYPES_TS) ? [TYPES_TS] : [],
    })

    if (result.errors.length > 0) {
      console.error(`Settings parity check failed with ${result.errors.length} error(s):`)
      for (const err of result.errors) {
        console.error(`  - ${err}`)
      }
      process.exitCode = 1
    } else {
      console.log('Settings parity OK')
    }
  }
}
