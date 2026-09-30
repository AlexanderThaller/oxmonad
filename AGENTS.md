# AGENTS

## Development

- Rust project
- Run cargo fmt after changes
- Run cargo clippy after changes and fix issues
- Make bigger changes in a worktree keep main clean in that case. Bigger
  changes include changes that require working on more than one rust
  code change that could lead to the compilation failing.

## Git Commit Messages

### Format

```
<type>(optional-scope): short description
```

Conventional Commits create a clear, machine-readable history.

They make changelogs automatic and semantic versioning possible — and
they make a Git log scannable.

### Examples

```
feat(dashboard): add survey results overview
fix(chatbot): handle empty LLM response
docs(readme): add local setup instructions
chore(ci): add linting workflow
refactor(api): simplify survey submission handler
test(chatbot): add timeout handling tests
```

### Commit Types

| Type     | Meaning                                    |
| -------- | ------------------------------------------ |
| feat     | A new feature.                             |
| fix      | A bug fix.                                 |
| docs     | Documentation only.                        |
| style    | Formatting only — no logic change.         |
| refactor | Code change without feature or bug change. |
| test     | Adding or updating tests.                  |
| chore    | Tooling, dependencies, CI/CD.              |
| ci       | CI/CD pipeline changes.                    |
| perf     | Performance improvement.                   |
