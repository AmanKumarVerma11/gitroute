# gitroute

A macOS menu bar app that keeps several GitHub accounts logged in at once and
uses the right one for every git operation, based on which folder the repo
lives in.

- Folder rules: `~/Code/work/` → work account, everything else → default.
  The closest matching rule wins.
- A git credential helper (`git-credential-gitroute`) hands git the right
  token over HTTPS. Never prompts.
- Generated `includeIf` config so commits carry the right name and email.
- `gitroute which <path>` explains which account a repo uses, and why.

**Status:** early development, not usable yet.

## Layout

| Crate | What it is |
|---|---|
| `crates/gitroute-core` | Config, resolver, GitHub API, token store, gitconfig writer |
| `crates/git-credential-gitroute` | The credential helper git calls |
| `crates/gitroute-cli` | The `gitroute` command |
| `crates/gitroute-app` | Menu bar app and settings window |

## Build

```sh
cargo build
cargo test
```

## License

MIT
