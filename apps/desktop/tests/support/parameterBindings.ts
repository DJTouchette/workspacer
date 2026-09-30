import ts from 'typescript';
/** Caller-object provenance, not a scan of every nearby identifier. This keeps
 * rename source keys, nested binding/type/member paths, and computed literals;
 * comments and unrelated objects cannot manufacture coverage. */
export function desktopParameterFields(
  handler: ts.Node,
  source: ts.SourceFile,
  topLevelOnly = false,
): string[] {
  while (ts.isParenthesizedExpression(handler) || ts.isCallExpression(handler)) {
    if (ts.isParenthesizedExpression(handler)) handler = handler.expression;
    else {
      if (!ts.isIdentifier(handler.expression) || handler.arguments.length !== 1)
        throw Error('unresolved capability handler wrapper');
      handler = handler.arguments[0];
    }
  }
  if (!ts.isArrowFunction(handler) && !ts.isFunctionExpression(handler))
    throw Error('unresolved capability handler body: ' + handler.getText(source));
  const aliases = new Map<string, string[]>(),
    fields = new Set<string>();
  const first = handler.parameters[0];
  if (!first) return [];
  const add = (parts: string[]): void => {
    for (const part of topLevelOnly ? parts.slice(0, 1) : parts) if (part) fields.add(part);
  };
  const typeFields = (type: ts.TypeNode | undefined, prefix: string[] = []): void => {
    if (!type) return;
    if (topLevelOnly && prefix.length) {
      add(prefix);
      return;
    }
    if (ts.isTypeLiteralNode(type))
      for (const member of type.members) {
        if (ts.isPropertySignature(member) && member.name) {
          if (ts.isIdentifier(member.name) || ts.isStringLiteral(member.name)) {
            fields.add(member.name.text);
            typeFields(member.type, [...prefix, member.name.text]);
          } else throw Error('unresolved caller type property');
        }
      }
    else if (ts.isUnionTypeNode(type) || ts.isIntersectionTypeNode(type))
      type.types.forEach((part) => typeFields(part, prefix));
    else if (ts.isArrayTypeNode(type)) typeFields(type.elementType, prefix);
  };
  const provenance = (expr: ts.Expression): string[] | undefined => {
    if (ts.isIdentifier(expr)) return aliases.get(expr.text);
    if (ts.isParenthesizedExpression(expr) || ts.isNonNullExpression(expr))
      return provenance(expr.expression);
    if (ts.isAsExpression(expr) || ts.isTypeAssertionExpression(expr))
      return provenance(expr.expression);
    if (
      ts.isBinaryExpression(expr) &&
      [ts.SyntaxKind.QuestionQuestionToken, ts.SyntaxKind.BarBarToken].includes(
        expr.operatorToken.kind,
      )
    )
      return provenance(expr.left);
    if (ts.isPropertyAccessExpression(expr)) {
      const base = provenance(expr.expression);
      return base && [...base, expr.name.text];
    }
    if (ts.isElementAccessExpression(expr)) {
      const base = provenance(expr.expression);
      if (!base) return undefined;
      if (ts.isStringLiteral(expr.argumentExpression))
        return [...base, expr.argumentExpression.text];
      if (ts.isNumericLiteral(expr.argumentExpression)) return base;
      throw Error('unresolved computed caller parameter');
    }
    return undefined;
  };
  const pattern = (name: ts.BindingName, prefix: string[]): void => {
    if (ts.isIdentifier(name)) {
      aliases.set(name.text, prefix);
      return;
    }
    if (ts.isArrayBindingPattern(name)) {
      for (const element of name.elements)
        if (ts.isBindingElement(element)) pattern(element.name, prefix);
      return;
    }
    for (const element of name.elements) {
      if (element.dotDotDotToken) {
        pattern(element.name, prefix);
        continue;
      }
      const key = element.propertyName || element.name;
      if (!ts.isIdentifier(key) && !ts.isStringLiteral(key))
        throw Error('unresolved caller binding key');
      const next = [...prefix, key.text];
      add(next);
      pattern(element.name, next);
    }
  };
  pattern(first.name, []);
  typeFields(first.type);
  const visit = (node: ts.Node): void => {
    if (ts.isVariableDeclaration(node) && node.initializer) {
      const value = provenance(node.initializer);
      if (value) {
        pattern(node.name, value);
        typeFields(node.type, value);
        let initial: ts.Expression = node.initializer;
        while (
          ts.isParenthesizedExpression(initial) ||
          ts.isAsExpression(initial) ||
          ts.isTypeAssertionExpression(initial)
        ) {
          if (ts.isAsExpression(initial) || ts.isTypeAssertionExpression(initial))
            typeFields(initial.type, value);
          initial = initial.expression;
        }
      }
    }
    if (ts.isPropertyAccessExpression(node) || ts.isElementAccessExpression(node)) {
      const value = provenance(node);
      if (value) add(value);
    }
    ts.forEachChild(node, visit);
  };
  visit(handler.body);
  return [...fields].sort();
}
