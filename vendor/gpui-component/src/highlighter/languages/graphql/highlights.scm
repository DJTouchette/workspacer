; Workspacer's highlights for tree-sitter-graphql 0.1.0, which ships no query.

[
  "query"
  "mutation"
  "subscription"
  "fragment"
  "on"
  "type"
  "interface"
  "union"
  "enum"
  "input"
  "scalar"
  "schema"
  "extend"
  "implements"
  "directive"
  "repeatable"
] @keyword

(operation_type) @keyword

[
  "QUERY"
  "MUTATION"
  "SUBSCRIPTION"
  "FIELD"
  "FRAGMENT_DEFINITION"
  "FRAGMENT_SPREAD"
  "INLINE_FRAGMENT"
  "VARIABLE_DEFINITION"
  "SCHEMA"
  "SCALAR"
  "OBJECT"
  "FIELD_DEFINITION"
  "ARGUMENT_DEFINITION"
  "INTERFACE"
  "UNION"
  "ENUM"
  "ENUM_VALUE"
  "INPUT_OBJECT"
  "INPUT_FIELD_DEFINITION"
] @constant

(named_type (name) @type)

(operation_definition (name) @function)
(fragment_name (name) @function)
(directive_definition (name) @attribute)
(directive "@" @attribute (name) @attribute)

(field (name) @property)
(alias (name) @property)
(field_definition (name) @property)
(object_field (name) @property)
(argument (name) @variable.special)
(input_value_definition (name) @variable.special)
(variable) @variable.special

(enum_value) @constant
(boolean_value) @boolean
(null_value) @constant
[
  (int_value)
  (float_value)
] @number

(description) @comment.doc
(string_value) @string
(comment) @comment

[
  "!"
  "="
  "&"
  "|"
  "..."
] @operator

[
  "("
  ")"
  "["
  "]"
  "{"
  "}"
] @punctuation.bracket

[
  ":"
  (comma)
] @punctuation.delimiter
