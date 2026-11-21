// 逐函数统计 TypeScript 圈复杂度，计数点与 ESLint 内置 complexity 规则（classic）一致：
// catch、三元、&& || ??、for/for-in/for-of/while/do、if、默认值、带条件的 case、逻辑赋值、可选链。
// 输出 JSON 供 crap_report.py 与 Vitest 覆盖率合并；嵌套函数各自计数，不计入外层
import { readFileSync } from 'node:fs'
import { createRequire } from 'node:module'
import { pathToFileURL } from 'node:url'

const require = createRequire(import.meta.url)
const ts = require('typescript')

const FUNCTION_KINDS = new Set([
  ts.SyntaxKind.FunctionDeclaration,
  ts.SyntaxKind.FunctionExpression,
  ts.SyntaxKind.ArrowFunction,
  ts.SyntaxKind.MethodDeclaration,
  ts.SyntaxKind.Constructor,
  ts.SyntaxKind.GetAccessor,
  ts.SyntaxKind.SetAccessor,
])

const DECISION_KINDS = new Set([
  ts.SyntaxKind.CatchClause,
  ts.SyntaxKind.ConditionalExpression,
  ts.SyntaxKind.ForStatement,
  ts.SyntaxKind.ForInStatement,
  ts.SyntaxKind.ForOfStatement,
  ts.SyntaxKind.IfStatement,
  ts.SyntaxKind.WhileStatement,
  ts.SyntaxKind.DoStatement,
  ts.SyntaxKind.CaseClause,
])

const BRANCHING_OPERATORS = new Set([
  ts.SyntaxKind.AmpersandAmpersandToken,
  ts.SyntaxKind.BarBarToken,
  ts.SyntaxKind.QuestionQuestionToken,
  ts.SyntaxKind.AmpersandAmpersandEqualsToken,
  ts.SyntaxKind.BarBarEqualsToken,
  ts.SyntaxKind.QuestionQuestionEqualsToken,
])

function isDecision(node) {
  if (DECISION_KINDS.has(node.kind)) return true
  if (ts.isBinaryExpression(node)) return BRANCHING_OPERATORS.has(node.operatorToken.kind)
  // ESTree 的 AssignmentPattern：参数与解构绑定的默认值
  if ((ts.isParameter(node) || ts.isBindingElement(node)) && node.initializer) return true
  if (ts.isShorthandPropertyAssignment(node) && node.objectAssignmentInitializer) return true
  // ESTree 只把带 ?. 的那一环标为 optional
  if (ts.isPropertyAccessExpression(node) || ts.isElementAccessExpression(node) || ts.isCallExpression(node)) {
    return Boolean(node.questionDotToken)
  }
  return false
}

function functionName(node) {
  if (node.name && ts.isIdentifier(node.name)) return node.name.text
  if (node.name && (ts.isStringLiteral(node.name) || ts.isPrivateIdentifier(node.name))) return node.name.text
  if (ts.isConstructorDeclaration(node)) return 'constructor'
  const parent = node.parent
  if (parent && ts.isVariableDeclaration(parent) && ts.isIdentifier(parent.name)) return parent.name.text
  if (parent && ts.isPropertyAssignment(parent) && ts.isIdentifier(parent.name)) return parent.name.text
  if (parent && ts.isPropertyDeclaration(parent) && ts.isIdentifier(parent.name)) return parent.name.text
  return '<anonymous>'
}

/** 返回文件内每个函数的复杂度与源码范围（行 1 起、列 0 起，与 istanbul 一致） */
export function analyzeSource(fileName, text) {
  const source = ts.createSourceFile(fileName, text, ts.ScriptTarget.Latest, true, ts.ScriptKind.TS)
  const position = (offset) => {
    const { line, character } = source.getLineAndCharacterOfPosition(offset)
    return { line: line + 1, column: character }
  }
  const functions = []
  const stack = []

  function visit(node) {
    const isFunction = FUNCTION_KINDS.has(node.kind) && node.body !== undefined
    if (isFunction) {
      const entry = {
        name: functionName(node),
        start: position(node.getStart(source)),
        end: position(node.getEnd()),
        complexity: 1,
      }
      functions.push(entry)
      stack.push(entry)
    } else if (stack.length && isDecision(node)) {
      stack[stack.length - 1].complexity += 1
    }
    // 参数默认值属于函数自身
    if (isFunction) {
      for (const parameter of node.parameters) {
        if (isDecision(parameter)) stack[stack.length - 1].complexity += 1
      }
    }
    ts.forEachChild(node, (child) => {
      if (isFunction && ts.isParameter(child)) {
        ts.forEachChild(child, visit)
        return
      }
      visit(child)
    })
    if (isFunction) stack.pop()
  }

  visit(source)
  return functions
}

export function analyzeFiles(files) {
  return files.map((file) => ({ file, functions: analyzeSource(file, readFileSync(file, 'utf8')) }))
}

if (import.meta.url === pathToFileURL(process.argv[1] ?? '').href) {
  process.stdout.write(`${JSON.stringify(analyzeFiles(process.argv.slice(2)))}\n`)
}
