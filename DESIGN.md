---
name: gitroute
description: Transit lines for your GitHub accounts. Every account is a line, and every repo's account is a route you can see.
colors:
  ground-light: "#F3F4F6"
  sidebar-light: "#E8EAED"
  card-light: "#FFFFFF"
  field-light: "#F7F8FA"
  hairline-light: "#DADDE2"
  ink-light: "#16181D"
  ink-soft-light: "#585F6A"
  ink-faint-light: "#A0A6AF"
  ok-light: "#188038"
  warn-light: "#A85600"
  danger-light: "#BE2C26"
  ground-dark: "#141518"
  sidebar-dark: "#1B1D21"
  card-dark: "#1F2226"
  field-dark: "#17191C"
  hairline-dark: "#30343A"
  ink-dark: "#EEF0F2"
  ink-soft-dark: "#A6ADB7"
  ink-faint-dark: "#626973"
  ok-dark: "#4CC26C"
  warn-dark: "#E8A840"
  danger-dark: "#F27066"
  line-green: "#00933C"
  line-blue: "#0039A6"
  line-orange: "#E85200"
  line-purple: "#B933AD"
  line-red: "#EE352E"
  line-yellow: "#FCCC0A"
  line-brown: "#996633"
  line-teal: "#00ADD0"
  line-gold-light: "#B08400"
  line-teal-light: "#008CAA"
  line-blue-dark: "#4682FF"
  line-unknown: "#808183"
typography:
  heading:
    fontFamily: "Helvetica Neue Medium"
    fontSize: "26px"
  title:
    fontFamily: "Helvetica Neue Medium"
    fontSize: "17px"
  body:
    fontFamily: "Helvetica Neue"
    fontSize: "14px"
  button:
    fontFamily: "Helvetica Neue Medium"
    fontSize: "13.5px"
  small:
    fontFamily: "Helvetica Neue"
    fontSize: "12px"
  mono:
    fontFamily: "SF Mono"
    fontSize: "13px"
  code:
    fontFamily: "SF Mono"
    fontSize: "34px"
rounded:
  control: "8px"
  banner: "10px"
  card: "12px"
  window: "14px"
  chip: "999px"
spacing:
  item: "8px"
  card-padding: "18px"
  content-left: "36px"
  between-cards: "12px"
  after-header: "22px"
components:
  button:
    backgroundColor: "#E5E7EB"
    textColor: "{colors.ink-light}"
    typography: "{typography.button}"
    rounded: "{rounded.control}"
    padding: "5px 12px"
    height: "28px"
  button-primary:
    backgroundColor: "{colors.ink-light}"
    textColor: "{colors.ground-light}"
    typography: "{typography.button}"
    rounded: "{rounded.control}"
  button-danger:
    textColor: "{colors.danger-light}"
    typography: "{typography.button}"
  card:
    backgroundColor: "{colors.card-light}"
    rounded: "{rounded.card}"
    padding: "18px"
  sidebar:
    backgroundColor: "{colors.sidebar-light}"
    width: "236px"
  chip:
    textColor: "{colors.ink-soft-light}"
    typography: "{typography.small}"
    rounded: "{rounded.chip}"
    padding: "2px 9px"
---

# Design System: gitroute

## Overview

gitroute shows multi-account git routing as a transit system. Every GitHub account is a line, with a color and a lettered round bullet like a metro line. A folder rule is a station on its account's line. "Check a path" draws the actual route from the repo's folders down to the account it ends at. Everything that isn't a line stays neutral, so color always means "which account".

This is an Operate surface in a native macOS window: a sidebar plus content, opened often to glance at status. The world shows up in the details (bullets, the sidebar drawn as a line, route diagrams), never at the cost of plain labels. Light and dark follow the system appearance.

## Colors

The strategy is restrained: cool neutral grounds and ink controls, plus a fixed set of classic transit line colors that only ever mean "an account".

### Primary

Ink (`ink-light` / `ink-dark`) is the action color. There is exactly one filled ink button per area: Install…, Add account, Add folder…, Log in again, and the confirm button in dialogs.

### Neutral

Each appearance has five grounds: ground (window), sidebar (one step darker in light, lighter in dark), card, field (text inputs and diff box) and hairline (borders and separators). There are three text inks: ink for content, ink-soft for secondary text and labels, ink-faint for disabled text and the unlit rails of diagrams. ok, warn and danger are status colors, always paired with text.

### Named Rules

- **The Line Rule.** Line colors appear only on account bullets, the colored part of a route, and the rule station's ring. Never on buttons, backgrounds, text or decoration.
- **Assigned by order.** Accounts get line colors by their position in the config (green, blue, orange, purple, red, yellow, brown, teal, then repeating). The bullet letter is the first letter of the login not already used by another account, so lines never differ by color alone.
- **Three to one, both ways.** Every line holds at least 3:1 against the card it sits on, in each appearance: yellow becomes gold and teal deepens on light cards, and blue lifts on dark ones. Bullet letters are white unless white falls under 3:1 on that line, then black. A unit test enforces both.
- **Hollow means default.** A hollow bullet (a ring with a colored letter) means "this folder gets that line through the default", as opposed to a rule naming it directly.

## Typography

Helvetica Neue, the lettering of transit signage, set from the system copy at `/System/Library/Fonts/HelveticaNeue.ttc`. Faces are looked up by name: Regular, Medium and Bold. SF Mono is used for paths, git output and the device code, and only for those. egui's bundled fonts stay behind as the fallback.

### Hierarchy

- **Heading** (26 px, Medium): one per page, the page name.
- **Title** (17 px, Medium): account logins, card titles, dialog titles.
- **Body** (14 px, Regular): everything else. Intro lines and labels use ink-soft.
- **Button** (13.5 px, Medium).
- **Small** (12 px): metadata such as "Signed in", times and chip text.
- **Mono** (13 px): paths and diffs. **Code** (34 px mono): the GitHub device code only.

## Layout

The window is a fixed 236 px sidebar plus content. Content starts 36 px from the sidebar, caps at 700 px wide, and scrolls vertically. Each page is a heading, then a one-line ink-soft intro, 22 px of space, then cards stacked 12 px apart. The sidebar holds the wordmark, the navigation line, and the install status card pinned to its foot. The default window is 940×640, the minimum 760×480.

## Elevation & Depth

Flat surfaces separated by 1 px hairlines. Only floating things cast shadows: dialogs (12 px offset, 36 px blur, low-alpha black) and popups (6 px offset, 18 px blur). Dialogs dim the window behind them.

## Shapes

Controls have an 8 px radius, banners 10, cards 12 and windows 14; chips are fully round. Bullets, stations and status dots are circles. Diagram rails are straight 2–3 px lines (unlit) or 6 px lines (an account's route).

## Components

### Buttons

The default button is a soft neutral fill with ink text. The primary button is ink-filled with ground-colored text, at most one per area. A page's main action ("Add account…") sits at the trailing edge of the page header. Destructive buttons ("Remove account…", "Remove") are danger-colored text with no fill until hovered, at the far trailing edge of their row.

In a destructive confirmation (Uninstall, Remove account), Cancel is the filled default, the destructive choice is a neutral button with a red label, and Esc or a click outside cancels. Non-destructive confirmations (Install) keep their action as the default.

### Chips

Round outline chips in ink-soft small text label states: "Default", "folder rule", "rule: use default", "no rule: default account".

### Cards / Containers

White (or dark card) with a 1 px hairline, 12 px radius and 18 px padding. One card per account, one per list (rules, activity) and one per flow (add account, route). Cards never nest.

### Inputs / Fields

The field fill and 8 px radius. Each input is named by its visible label (`labelled_by`), so VoiceOver reads it correctly. Paths are typed in mono. Dropdowns show a small up/down chevron pair in ink-soft, as on macOS pop-up buttons, never egui's filled triangle.

### Navigation

The sidebar's four pages are drawn as stations on a vertical rail. The current page is a filled ink station with a ring; the others are hollow. Labels are body size, Medium and ink when current, Regular and ink-soft otherwise. Each station is a Tab with an accessible name.

### Account line

Under each account's name plate, a 4 px rail in the line color drops out of the bullet and runs right.
- A station for each folder routed onto it: a filled pip when a rule names the account, hollow when it rides the default.
- On the default's line, a hollow "everything else" terminus.
- More than five stations collapse to "+N more". With none, a stub reads "No folders routed here yet".

### Route diagram (signature)

"Check a path" lists the path's folders top to bottom, from `~` (or `/`) to the repo, each on a rail.
- Above the deciding rule, the rail is thin and faint, with hollow stations.
- The rule's folder gets a ring in the account's color and a chip.
- From there down, the rail is the account's 6 px line with small light pips.
- It ends in a 30 px bullet with the login and "commits as Name <email>".
- With no rule, the default's line runs the whole way and the first station gets a "no rule: default account" chip.
- Below, the plain-text explanation in mono, with `~` paths.

## Do's and Don'ts

### Do:

- Give every account its bullet wherever the account is named in a list or diagram.
- Pair every status color with words ("Installed", "Needs re-login: GitHub rejected its token").
- Keep destructive actions as quiet text at the far edge, and confirm them in a dialog.
- Shorten long paths on `/` boundaries, keeping more of the end (it's what tells rules apart), with the full path on hover. Where only the folder matters (the helper in the sidebar), show the folder.
- In a one-line explanation, shorten the paths, never the conclusion: `→ amannnsyn` always stays visible.
- Hide a page's header action while its own flow is running (no "Add account…" during a sign-in).
- Show problem states as problems: an install that points git at another helper is amber, not green.

### Don't:

- Use line colors for anything other than accounts and their routes.
- Put a second filled ink button in the same area.
- Use glyph characters for icons. Bullets, stations, dots and the mark are drawn.
- Use mono for anything that isn't a path, git output or a code.
