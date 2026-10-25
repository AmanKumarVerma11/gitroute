# Product

<!-- impeccable:product-schema 1 -->

## Platform

macos (native desktop app: a menu bar item plus a settings window, built
with Rust, egui/eframe and tray-icon. Not one of the schema's web/ios/android
values.)

## Users

People who use more than one GitHub account on one Mac, typically a
personal account plus a work account, and want git to use the right one
without thinking. Two audiences matter equally:
- the author, who uses it daily;
- strangers who install it from the open-source README and have never seen
  it before, so first-run clarity matters as much as daily use.

The window is opened often, not just for setup: people check which account
a repo uses and what the credential helper has been doing, so status has to
read at a glance.

## Product Purpose

Keep several GitHub accounts logged in at once and have every git operation
use the right one, decided by which folder the repo lives in. Make that
routing visible: which account a path uses, why, and what happened
recently. Success means zero password prompts, the correct commit identity
in every folder, and the user always able to see why.

## Positioning

Folder rules, not a global "active account" switch: the closest folder rule
wins, folders inherit from their parent, and the default covers everything
else. The same rule decides both the HTTPS token and the commit name/email,
and the app can explain every decision.

## Operating Context

- Lives in the macOS menu bar (no Dock icon). The settings window opens from
  the menu; closing it keeps gitroute running.
- Works alongside the user's existing `~/.gitconfig`, adding one marked
  block. Shows the exact diff before changing it, and keeps a backup.
- Accounts are added with GitHub's device flow: a code shown in the app,
  entered on github.com.
- A CLI (`gitroute`) does everything the window does.

## Capabilities and Constraints

- Accounts: add (device flow), make default, remove, edit commit name, pick
  the commit email from verified emails, re-login when GitHub rejects a
  token.
- Folder rules: add with the native folder picker, change account or "use
  default", remove.
- Check a path: explains which account and why, e.g.
  `/x/y → rule /x/ → amannnsyn`.
- Activity: the credential helper's recent decisions, never tokens.
- Install/uninstall into git, with a diff confirmation.
- v1 is github.com over HTTPS only. SSH, import of hand-made setups, and
  Windows/Linux are v2.
- Terminology: "account" (GitHub login), "folder rule", "default account",
  "needs re-login", "credential helper", "install".

## Brand Commitments

- Name: gitroute, lowercase. MIT-licensed open source.
- The user asked for Apple's design principles (the apple-design skill) and
  for it to feel native to macOS but distinctive, with its own recognizable
  identity rather than a System Settings clone.
- Menu bar icon: a small template-image branch glyph.

## Evidence on Hand

Real accounts and folders on the author's machine (personal
AmanKumarVerma11, work amannnsyn). There are no users, testimonials or
metrics yet, and none should be invented.

## Product Principles

1. Show why, not just what: every routing decision can be explained.
2. Never surprise: show the diff before touching git config, and confirm
   before removing anything.
3. Status first: problems (needs re-login, not installed) are visible at a
   glance, with the fix one click away.
4. Self-explanatory for a first-time user, without slowing down a daily one.

## Accessibility & Inclusion

Every control has an accessible name (enforced by the UI tests). Status is
never shown by color alone; it always has text.
