import type { CodeLanguage } from "@/lib/api/types";

const KEYWORDS: Record<string, string[]> = {
  python: ["def", "return", "if", "elif", "else", "for", "while", "in", "and", "or", "not", "import", "from", "as", "class", "with", "try", "except", "finally", "lambda", "yield", "pass", "break", "continue", "None", "True", "False", "is", "raise"],
  javascript: ["function", "return", "if", "else", "for", "while", "const", "let", "var", "class", "new", "import", "export", "from", "async", "await", "try", "catch", "throw", "null", "true", "false", "of", "in", "switch", "case", "break", "continue"],
  java: ["public", "private", "static", "void", "class", "return", "if", "else", "for", "while", "new", "int", "long", "boolean", "final", "import", "try", "catch", "throw", "null", "true", "false", "extends", "implements"],
  c: ["int", "char", "void", "return", "if", "else", "for", "while", "struct", "typedef", "const", "static", "sizeof", "NULL", "break", "continue", "include", "define"],
  rust: ["fn", "let", "mut", "pub", "struct", "enum", "impl", "match", "if", "else", "for", "while", "loop", "return", "use", "mod", "as", "in", "true", "false", "self", "Self", "const", "trait", "where"],
  go: ["func", "return", "if", "else", "for", "range", "var", "const", "type", "struct", "interface", "package", "import", "go", "defer", "map", "chan", "nil", "true", "false", "switch", "case"],
  sql: ["select", "from", "where", "join", "left", "right", "inner", "outer", "on", "group", "by", "order", "having", "insert", "into", "values", "update", "set", "delete", "create", "table", "index", "and", "or", "not", "null", "as", "limit", "distinct", "count"],
  bash: ["if", "then", "else", "fi", "for", "do", "done", "while", "case", "esac", "function", "echo", "export", "cd", "ls"],
};
KEYWORDS.typescript = [...KEYWORDS.javascript!, "interface", "type", "enum", "readonly", "number", "string", "boolean"];
KEYWORDS.cpp = [...KEYWORDS.c!, "class", "namespace", "template", "using", "new", "delete", "auto", "bool", "public", "private", "true", "false", "std"];
KEYWORDS.csharp = [...KEYWORDS.java!, "namespace", "using", "var", "string", "bool"];
KEYWORDS.kotlin = ["fun", "val", "var", "class", "return", "if", "else", "for", "while", "when", "null", "true", "false", "object", "import"];
KEYWORDS.swift = ["func", "let", "var", "class", "struct", "return", "if", "else", "for", "while", "guard", "nil", "true", "false", "import"];
KEYWORDS.php = ["function", "return", "if", "else", "foreach", "for", "while", "class", "new", "echo", "null", "true", "false", "public", "private"];

export type Token = { text: string; kind: "kw" | "num" | "comment" | "plain" };

const LINE_COMMENT: Partial<Record<CodeLanguage, RegExp>> = {
  python: /#.*/y, bash: /#.*/y, yaml: /#.*/y, sql: /--.*/y,
};

/** A small tokenizer: keywords, numbers and comments, which is all the design colours. */
export function highlight(source: string, language: CodeLanguage): Token[] {
  const words = new Set(KEYWORDS[language] ?? []);
  const comment = LINE_COMMENT[language] ?? /\/\/.*/y;
  const tokens: Token[] = [];
  const push = (text: string, kind: Token["kind"]) => {
    const last = tokens[tokens.length - 1];
    if (last && last.kind === kind) last.text += text;
    else tokens.push({ text, kind });
  };
  let i = 0;
  while (i < source.length) {
    comment.lastIndex = i;
    const cm = comment.exec(source);
    if (cm && cm.index === i) { push(cm[0], "comment"); i += cm[0].length; continue; }
    const word = /[A-Za-z_][A-Za-z0-9_]*/y;
    word.lastIndex = i;
    const w = word.exec(source);
    if (w && w.index === i) { push(w[0], words.has(language === "sql" ? w[0].toLowerCase() : w[0]) ? "kw" : "plain"); i += w[0].length; continue; }
    const num = /\d+(\.\d+)?/y;
    num.lastIndex = i;
    const nm = num.exec(source);
    if (nm && nm.index === i) { push(nm[0], "num"); i += nm[0].length; continue; }
    push(source[i]!, "plain");
    i++;
  }
  return tokens;
}
