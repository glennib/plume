# DuckDB v2 parser extensibility

Research date: 2026-09-27.
Source inspected: `duckdb/duckdb` branch `v2.0-cyanoptera`, HEAD `d8a1bd4f` (2026-09-25).

## Summary

DuckDB v2 replaces the Postgres-derived parser with its own PEG parser, and extensions can add grammar rules to it.
The mechanism is **C++-only**: there is no grammar registration in the stable C API.
An extension that adds syntax is therefore built against DuckDB's C++ internals
and is tied to the exact DuckDB version it was built for.
The API is also still changing before v2.0 GA.

## Grammar extensions (PEG)

Headers: `src/include/duckdb/parser/grammar_extension.hpp`, `src/include/duckdb/parser/grammar_change.hpp`.

- Subclass `GrammarExtension(name, description)` and implement `vector<GrammarChange> GetChanges() const`.
- Register it with `GrammarExtension::Register(DatabaseInstance &, shared_ptr<GrammarExtension>)`.
- `GrammarChange` constructors:
  - `AddRule(rule_definition, transform)`: a new PEG rule, e.g. `"PipeStage <- '|>' PipeOperator"`.
  - `AddChoice` / `PrependChoice(rule_name, choice)`: add an alternative to an existing rule.
    Prepended choices are tried first.
  - `RemoveChoice`, `ReplaceChoice`, `ReplaceRule`.
  - `SetTransformProcess(rule_name, transform)`: attach a transform to an existing rule.
  - `AddTerminalRuleOverride(rule_name, matcher_factory)`: custom terminal matching (tokenizer-level).
- A transform receives the `PEGTransformer` and the rule's `ParseResult`, and produces ordinary AST nodes
  (`SelectStatement`, `TableRef`, `ParsedExpression`, ...).
  It can call back into the transformer for built-in rules such as `TargetList`, `WhereClause` or `FromClause`,
  so standard SQL fragments are reused rather than re-parsed.
- New keywords are added by extending the keyword rules, e.g. `AddChoice("ReservedKeyword", "'EXTEND'")`.
- Grammar extensions are activated per connection: `SET active_grammar_extensions = ['<name>']`
  (local-scoped `VARCHAR[]` setting).
  Registered extensions are listed by the `duckdb_grammar_extensions()` table function.

### Reference example

`test/extension/loadable_grammar_extension_demo.cpp` is a loadable extension adding BigQuery-style pipe syntax
(`FROM t |> WHERE … |> AGGREGATE … |> ORDER BY …`).
It prepends `PipeSelectAtom` to `SelectAtom` and folds each stage into a `SelectStatement`.
It uses the C++ entry point `DUCKDB_CPP_EXTENSION_ENTRY`.
`test/extension/test_loadable_grammar_extension.test` exercises it.

The 2026-08-20 blog post shows a different API shape
(`ParserExtension.grammar_extension`, `select_atom_rule`, `RegisterSelectAtomTransformer`, `loader.RegisterKeyword`)
from the branch code above.
The post says the API may change before v2.0.

## Legacy parser extension hook

`src/include/duckdb/parser/parser_extension.hpp` still provides the older `ParserExtension`:

- `parse_function(info, tokens)`: called with the tokens of a statement the main parser rejected.
  Returns parse data and the number of tokens consumed.
- `plan_function(info, context, parse_data)`: plans the statement to a `TableFunction` plus parameters.
- `parser_override`: replace parsing of the whole query string.

This is what ggsql's DuckDB extension uses for `VISUALISE` today:
a whole-statement fallback with hand-rolled tokenization.
It is also C++-only.

## What the C API offers

`api_spec/v2/tokenizer/tokenizer.yaml` and `api_spec/v2/sql_statement/sql_statement.yaml` tokenize
and parse SQL "in the context of whatever grammar extensions are loaded on the given connection".
They consume grammar extensions but cannot define them.

## Implications for a VISUALIZE clause

- Syntax must come from a C++ grammar extension compiled against the target DuckDB version.
- The transform can desugar `<query> VISUALIZE …` into standard AST
  that calls functions registered by a separate extension on the stable C ABI, e.g. an aggregate over the query's rows.
  That keeps the version-locked part small.
- Users (or the shim's load step, if permitted) must enable the grammar via `active_grammar_extensions`.
- ggsql accepts both `VISUALISE` and `VISUALIZE`; a PEG alternation supports both at no cost.

## Sources

- [DuckDB v2.0: Your Database Deserves a Better Parser](https://duckdb.org/2026/08/20/duckdb-20-peg-parser)
- [PEG Parser documentation](https://duckdb.org/docs/current/sql/peg_parser)
- [the-stats-duck issue #46](https://github.com/KoliStat/the-stats-duck/issues/46): another project planning to move a
  `VISUALIZE` clause from the fallback hook to PEG grammar hooks on v2.
- [ggsql](https://github.com/posit-dev/ggsql),
  [ggsql alpha announcement](https://opensource.posit.co/blog/2026-04-20_ggsql_alpha_release/)
- DuckDB source: files named above.
