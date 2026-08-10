# The UI gate, answered

**Status:** this answers the open decision gate *"Granola UI description → blocks G9"*,
which sat at its default ("build from SPEC §9 as written") for the whole project.

**Evidence:** Granola's macOS app, observed directly on 2026-08-07 — the library, an
open meeting, a new note mid-recording, and the template menu. Screens were captured
from a running copy, not from marketing pages or an iOS reference. No meeting content
from those captures is reproduced here; only structure.

---

## 1. The finding

Oatmeal's UI is not a bad product interface. **It is not a product interface at all.**
Its own subtitle says so:

> Oatmeal `PHASE 5` — *Build harness. Each card proves one piece of the pipeline works
> end to end.*

That was the correct thing to build while proving the pipeline, and it did its job. But
every surface is a diagnostic: `Record` with Arm/Start/Stop, an `IDLE` badge and two
level meters; `SUMMARY` with a model dropdown, a large Regenerate button, the model
name and build time, and *Delete this version*; `NOTES` as a small empty box beside a
monospace `TRANSCRIPT`; `Tune linking (0 links)` along the bottom.

The gap between this and Granola is not styling. **One shows you its machinery. The
other shows you your meeting.**

---

## 2. Screen inventory

Granola has two screens. Not a sidebar, not a tab bar — two.

### 2.1 Library (home)

- **No sidebar.** A single centred column, roughly 570pt wide in an 1100pt window.
- **Chrome is two buttons**, top right: `Invite`, `+ New note`.
- **`Coming up`** — a serif display heading, then a calendar card: a large date numeral
  with month and weekday stacked beside it, then that day's events, each with a
  coloured left rule, title, and time beneath.
- **Meetings grouped by day** under small grey date headers (`Fri, Jul 24`). Each row:
  document icon, title, owner beneath, timestamp right-aligned.
- **Row controls appear on hover only** — a privacy pill and a `…` menu. Two captures
  seconds apart show them present and absent.
- **Chat is pinned to the bottom** as a floating pill: `Ask anything` plus one
  contextual suggestion chip.

### 2.2 Meeting (the document)

- **Top left:** one `‹ 🏠` pill. **Top right:** `…`, `Share`, link icon. That is the
  entire frame.
- **Title in serif, ~40px, wrapping to two lines** — the largest thing on screen.
- **One row of metadata pills**, then straight into content:
  - `≡` grouped with `✨ Enhanced ⌄` — template picker (see §3)
  - `📅 <date>` and `👥 <attendees>` in one pill
  - `📁 Add to folder`
- **Headings carry a `#` in the left gutter** — a quiet markdown affordance, recessive
  rather than decorative.
- **Two bullet levels**: filled `•` and brighter text at level one; hollow `○` and
  dimmer text at level two. Hierarchy is carried by *opacity*, not size.
- **Nothing is a card.** No borders, fills, or panels anywhere in the content. Text on
  background.
- **Floating bottom bar**: audio control, `Ask anything`, and one action chip
  (`Write follow up email`).
- **Centred footer microcopy**: an admission that the AI can be wrong.

### 2.3 Meeting (recording / empty)

This is the screen that most indicts Oatmeal's current design.

While recording, Granola shows **the note canvas, empty**, with the placeholder
*"Write notes, or press '/' for templates"* — and a small floating control carrying a
live level indicator, a chevron, and a stop square.

No Arm/Start/Stop trio. No status badge. No meters spanning the window. Recording
occupies perhaps 200 square points in the corner; the rest of the screen belongs to the
thing the user is actually doing.

Oatmeal presently gives recording an entire card with three buttons, a status pill and
two meters — **more visual weight than the notes.**

---

## 3. The template control

`✨ Enhanced ⌄` is not a view switcher. It opens a menu:

- `✨ Enhanced notes` — currently applied, with an inline `↻` and a `✓`
- **Templates**, each with an emoji: `1 to 1`, `Customer: Discovery`, `Hiring`,
  `Stand-Up`, `Weekly Team Meeting`
- `All templates…`, `+ New template`

Two lessons. **Templates are named for meeting types and picked inline**, one click from
the document. And **regenerate is a 16px icon inside a menu row**, not a button — where
Oatmeal spends a large filled orange button plus a separate select on the same function.

---

## 3b. The iOS app, for what it is worth

Mobbin's Granola iOS set was checked on 2026-08-07. **Four of its 112 screens are
visible without a paid account**; the rest are blurred behind Pro, so this is a
partial reading and the desktop captures above remain the primary reference —
which is right anyway, since Oatmeal is a desktop app and iOS would teach the wrong
layout.

What the free screens add:

- **The iOS app is light** (cream, roughly #f5f2ec) where the desktop app I captured
  was dark. Granola evidently follows the platform rather than owning one palette.
- **`Coming up` is confirmed as a real, prominent pattern**, not an artefact of one
  screenshot: a small grey label, then a card with a date chip — month above day,
  month in red — the event title, and its time.
- **Groups are labelled relatively**: "Earlier today", not a date. This matches the
  Today / Yesterday / date fallback G30 already implements.
- The chat entry sits bottom-centre again — `Ask anything` — beside a compose button
  for a new note.
- On iOS the meeting rows *are* cards, white on cream. On desktop they are
  borderless. The desktop treatment is the one Oatmeal follows.

Nothing here changes the plan. It is corroboration plus one detail worth having: the
date chip in `Coming up`, which is still unbuilt in G30.

---

## 4. Mapping

| Oatmeal today | Target |
|---|---|
| One page of diagnostic cards | Two screens: Library, Meeting |
| `Record` card: Arm/Start/Stop, `IDLE`, two meters | One floating control: level indicator + stop |
| `NOTES` box beside monospace `TRANSCRIPT` | Notes *are* the page, full measure |
| `Summary ⌄` select + large `Regenerate` button | `✨ <template> ⌄` pill with an inline `↻` |
| Model name, build time, *Delete this version* on the page | Behind `…` |
| `Tune linking (0 links)` on the page | Settings |
| Dense UI sans throughout | Serif display title, sans body, opacity hierarchy |
| Full-bleed stacked cards | Centred column, generous margins |
| Chat as a card | Floating bottom pill, always present |
| No way to browse meetings | Library grouped by day, `Coming up` on top |

---

## 5. Where Oatmeal must NOT copy Granola

**Granola has no note↔transcript link.** That is precisely what G17 and G18 exist to
build, and it is the product's reason to exist. So the transcript cannot simply be
demoted the way Granola demotes it.

The open question is how Granola surfaces raw transcript at all — presumably behind the
`≡` icon left of the template pill, which was not captured.

**Default if that stays unanswered:** do *not* ship a permanent split pane. Keep the
calm single-column document, and make the link a **hover affordance** — hovering a note
line reveals the moment it came from, and hovering a transcript line highlights the note
that cites it. G18 already implements this as a ProseMirror decoration; what changes is
that it stops living inside a second permanent panel.

This preserves Granola's calm while making Oatmeal's differentiator visible exactly when
it is relevant, which a permanent pane does not — a pane that is always there is
furniture, and furniture is ignored.

---

## 6. Implementation order

1. **Library screen.** There is currently no way to browse meetings at all; this is a
   missing feature, not a restyle.
2. **Meeting screen.** Serif title, metadata pills, notes as the full-width canvas.
3. **Collapse the Record card** into the floating control.
4. **Move the machinery** — model, regenerate, delete-version, link tuning — behind `…`
   and settings.
5. **Template pill** replacing the summary select and Regenerate button.
6. **Link-on-hover**, replacing the permanent transcript pane.
7. **Type and spacing pass** last, once the structure is right.

Steps 1 and 2 are the ones that change what the app *is*. The rest is subtraction.
